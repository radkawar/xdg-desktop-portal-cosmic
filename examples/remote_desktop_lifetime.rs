use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, bail, ensure};
use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::desktop::{PersistMode, ResponseError};
use clap::Parser;
use gst::prelude::*;

/// Exercise real portal consent, capture and retained input-FD revocation.
#[derive(Parser)]
struct Args {
    /// Use ScreenCast only, without requesting any input device.
    #[arg(long, conflicts_with = "input_text")]
    view_only: bool,
    /// Require a real local Deny/Cancel response instead of a started session.
    #[arg(long, conflicts_with = "input_text")]
    expect_cancel: bool,
    /// Position the local pointer over a test terminal, then type through the portal.
    #[arg(long)]
    input_text: Option<String>,
    /// Save the actual captured frame as a PNG at this path.
    #[arg(long)]
    frame_output: Option<String>,
}

async fn checkpoint(name: &str) -> anyhow::Result<()> {
    println!("{name}: press Enter on this client's stdin to continue");
    std::io::stdout().flush()?;
    let bytes = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)
    })
    .await??;
    ensure!(bytes != 0, "client stdin closed before {name}");
    Ok(())
}

fn expect_cancelled<T>(response: ashpd::Result<T>) -> anyhow::Result<()> {
    match response {
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => {
            println!("CANCELLED: real local portal response");
            Ok(())
        }
        Err(error) => Err(error.into()),
        Ok(_) => bail!("portal granted access despite expected local cancellation"),
    }
}

fn capture_frame(fd: OwnedFd, node: u32, frame_output: Option<&str>) -> anyhow::Result<()> {
    let sink = if frame_output.is_some() {
        "pngenc ! filesink name=sink"
    } else {
        "fakesink name=sink sync=false"
    };
    let pipeline = gst::parse::launch(&format!(
        "pipewiresrc fd={} path={node} num-buffers=1 ! video/x-raw,format=RGBA ! videoconvert ! video/x-raw,format=RGB ! identity name=witness signal-handoffs=true ! {sink}",
        fd.as_raw_fd()
    ))?
    .dynamic_cast::<gst::Pipeline>()
    .map_err(|_| anyhow::anyhow!("capture pipeline was not a pipeline"))?;
    if let Some(path) = frame_output {
        pipeline
            .by_name("sink")
            .context("missing PNG sink")?
            .set_property("location", path);
    }
    let witness = pipeline
        .by_name("witness")
        .context("missing frame witness")?;
    let frames = Arc::new(AtomicUsize::new(0));
    let received = frames.clone();
    witness.connect("handoff", false, move |_| {
        received.fetch_add(1, Ordering::Relaxed);
        None
    });
    pipeline.set_state(gst::State::Playing)?;
    let message = pipeline
        .bus()
        .context("missing capture bus")?
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(15),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
    let caps = witness
        .static_pad("sink")
        .and_then(|pad| pad.current_caps());
    pipeline.set_state(gst::State::Null)?;
    match message.as_ref().map(|message| message.view()) {
        Some(gst::MessageView::Eos(_)) => {}
        Some(gst::MessageView::Error(error)) => bail!("capture failed: {}", error.error()),
        _ => bail!("no real captured frame before deadline"),
    }
    ensure!(
        frames.load(Ordering::Relaxed) == 1,
        "capture delivered no frame"
    );
    println!(
        "CAPTURED_BUFFERS=1 CAPS={}",
        caps.context("missing video caps")?
    );
    Ok(())
}

fn assert_socket_open(stream: &mut UnixStream) -> anyhow::Result<()> {
    let mut bytes = [0; 1024];
    loop {
        match stream.read(&mut bytes) {
            Ok(0) => bail!("compositor input FD was already closed before Session.Close"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
}

async fn assert_socket_revoked(stream: &mut UnixStream) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = [0; 1024];
    loop {
        match stream.read(&mut bytes) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                ensure!(
                    Instant::now() < deadline,
                    "retained compositor input FD remained open after Session.Close"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => return Err(error.into()),
        }
    }
    match stream.write(b"late input") {
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::NotConnected
            ) =>
        {
            println!("INPUT_FD_REVOKED={:?}", error.kind())
        }
        other => bail!("retained input FD accepted a late write: {other:?}"),
    }
    Ok(())
}

async fn run_control(args: &Args) -> anyhow::Result<()> {
    let desktop = RemoteDesktop::new().await?;
    let screencast = Screencast::new().await?;
    let session = desktop.create_session().await?;
    desktop
        .select_devices(
            &session,
            DeviceType::Keyboard | DeviceType::Pointer,
            None,
            PersistMode::DoNot,
        )
        .await?
        .response()?;
    screencast
        .select_sources(
            &session,
            CursorMode::Embedded,
            SourceType::Monitor.into(),
            false,
            None,
            PersistMode::DoNot,
        )
        .await?
        .response()?;
    println!("AWAITING_NATIVE_CONSENT control");
    let response = desktop.start(&session, None).await?.response();
    if args.expect_cancel {
        expect_cancelled(response)?;
        checkpoint("CANCELLED_SESSION").await?;
        println!("PASS control cancellation");
        return Ok(());
    }
    let response = response?;
    ensure!(
        response
            .devices()
            .contains(DeviceType::Keyboard | DeviceType::Pointer),
        "input devices were not consented"
    );
    let streams = response.streams().context("no captured streams")?;
    ensure!(streams.len() == 1, "select exactly one display");
    let node = streams[0].pipe_wire_node_id();
    println!("NODE={node}");
    capture_frame(
        screencast.open_pipe_wire_remote(&session).await?,
        node,
        args.frame_output.as_deref(),
    )?;
    if let Some(text) = &args.input_text {
        checkpoint("INPUT_READY").await?;
        desktop
            .notify_pointer_button(&session, 0x110, KeyState::Pressed)
            .await?;
        desktop
            .notify_pointer_button(&session, 0x110, KeyState::Released)
            .await?;
        desktop.notify_pointer_motion(&session, 1.0, 0.0).await?;
        for character in text.chars().chain(std::iter::once('\n')) {
            let keysym = match character {
                '\n' => 0xff0d,
                character if u32::from(character) <= 0xff => u32::from(character),
                character => 0x01000000 | u32::from(character),
            } as i32;
            desktop
                .notify_keyboard_keysym(&session, keysym, KeyState::Pressed)
                .await?;
            desktop
                .notify_keyboard_keysym(&session, keysym, KeyState::Released)
                .await?;
        }
        println!("INPUT_SENT={text}");
    }
    checkpoint("CLOSE_READY").await?;
    let mut retained = UnixStream::from(desktop.connect_to_eis(&session).await?);
    retained.set_nonblocking(true)?;
    assert_socket_open(&mut retained)?;
    session.close().await?;
    assert_socket_revoked(&mut retained).await?;
    ensure!(
        desktop
            .notify_pointer_motion(&session, 1.0, 1.0)
            .await
            .is_err(),
        "portal accepted input after Session.Close"
    );
    // Keep the client, its D-Bus connection and its original FD alive while an
    // independent pw-dump observer verifies these exact NODE IDs disappear.
    checkpoint("CLOSED_FD_RETAINED").await?;
    println!("PASS control lifetime");
    Ok(())
}

async fn run_view(args: &Args) -> anyhow::Result<()> {
    let screencast = Screencast::new().await?;
    let session = screencast.create_session().await?;
    screencast
        .select_sources(
            &session,
            CursorMode::Embedded,
            SourceType::Monitor.into(),
            false,
            None,
            PersistMode::DoNot,
        )
        .await?
        .response()?;
    println!("AWAITING_NATIVE_CONSENT view (no input devices requested)");
    let response = screencast.start(&session, None).await?.response();
    if args.expect_cancel {
        expect_cancelled(response)?;
        checkpoint("CANCELLED_SESSION").await?;
        println!("PASS view cancellation");
        return Ok(());
    }
    let response = response?;
    ensure!(response.streams().len() == 1, "select exactly one display");
    let node = response.streams()[0].pipe_wire_node_id();
    println!("NODE={node}");
    capture_frame(
        screencast.open_pipe_wire_remote(&session).await?,
        node,
        args.frame_output.as_deref(),
    )?;
    checkpoint("CLOSE_READY").await?;
    session.close().await?;
    checkpoint("CLOSED_SESSION").await?;
    println!("PASS view lifetime");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    gst::init()?;
    let args = Args::parse();
    if args.view_only {
        run_view(&args).await
    } else {
        run_control(&args).await
    }
}
