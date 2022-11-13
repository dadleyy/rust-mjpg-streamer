#![forbid(unsafe_code)]

use std::io;

use clap::Parser;

use async_std::prelude::FutureExt;
use v4l::device::Device;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;

const BOUNDARY: &str = "mjpg-boundary";

#[derive(Parser, Debug)]
#[command(author, version = option_env!("RUSTY_MJPG_VERSION").unwrap_or_else(|| "dev"), about, long_about = None)]
struct CommandLineArguments {
  #[arg(short = 'd', long)]
  device: String,

  #[arg(short = 'a', long)]
  addr: String,
}

#[derive(Clone, Debug)]
struct SharedState {
  last_frame: async_std::sync::Arc<async_std::sync::RwLock<(Option<std::time::Instant>, Vec<u8>)>>,
  switchbox: async_std::channel::Sender<async_std::channel::Sender<()>>,
}

async fn snapshot(request: tide::Request<SharedState>) -> tide::Result<tide::Response> {
  let frame_reader = request.state().last_frame.read().await;

  let response = tide::Response::builder(200)
    .content_type("image/jpeg")
    .body(frame_reader.1.clone())
    .build();

  drop(frame_reader);

  Ok(response)
}

async fn stream(request: tide::Request<SharedState>) -> tide::Result<tide::Response> {
  // Create the channel whose receiver will be used as a async reader.
  let (writer, drain) = async_std::channel::bounded(2);
  let buf_drain = futures::stream::TryStreamExt::into_async_read(drain);

  // Create a channel that will be used for the video loop to tell us a new frame is ready.
  let (sender, receiver) = async_std::channel::unbounded();
  request.state().switchbox.send(sender).await?;

  // Prepare the response with the correct header
  let response = tide::Response::builder(200)
    .content_type(format!("multipart/x-mixed-replace;boundary={BOUNDARY}").as_str())
    .body(tide::Body::from_reader(buf_drain, None))
    .build();

  // In a separate task, continously check our shared buffer's timestamp. If that value differs
  // from the timestamp of the last message sent on our end, send a new multipart chunk.
  async_std::task::spawn(async move {
    let mut last_frame = None;
    let mut frame_count = 0;
    let mut last_debug = std::time::Instant::now();

    if let Err(error) = writer.send(Ok(format!("--{BOUNDARY}\r\n").as_bytes().to_vec())).await {
      log::error!("unable to write initial boundary - {error}");
      return;
    }

    loop {
      // Await for our semaphore that will let us know a frame is available.
      if let Err(error) = receiver.recv().await {
        log::warn!("unable to pull semaphore -  {error}");
        break;
      }

      // Calculate a milis timestamp that will be sent in our multipart chunk headers.
      let timestamp = match std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH) {
        Err(error) => {
          log::error!("unable to compute frame timestamp - {error}");
          break;
        }
        Ok(timestamp) => timestamp,
      }
      .as_millis();

      // Pull the mutext lock for read access and verify a timestamp is available and different
      // than our last.
      let frame_reader = request.state().last_frame.read().await;

      if frame_reader.0.is_none() {
        log::error!("no timestamp present on frame data, unexpected");
        drop(frame_reader);
        continue;
      }

      if last_frame.is_some() && last_frame == frame_reader.0 {
        log::warn!(
          "stale frame sent past semaphore ({last_frame:?} vs {:?}",
          frame_reader.0
        );
        drop(frame_reader);
        continue;
      }

      if last_frame.is_none() {
        log::debug!("initial frame, yay");
      }

      let now = std::time::Instant::now();
      frame_count += 1;

      if now.duration_since(last_debug).as_secs() > 3 {
        log::info!("{frame_count} frames in 3 seconds");
        last_debug = now;
        frame_count = 0;
      }

      // Start the buffer that we'll send using the boundary and some multi-part http header
      // context.
      let mut buffer = format!(
        "Content-Type: image/jpeg\r\nContent-Length: {}\r\nX-Timestamp: {}\r\n\r\n",
        frame_reader.1.len(),
        timestamp,
      )
      .into_bytes();

      // Actually push the JPEG data into our buffer.
      buffer.extend_from_slice(frame_reader.1.as_slice());
      buffer.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());

      last_frame = (*frame_reader).0;

      if let Err(error) = writer.send(Ok(buffer)).await {
        log::warn!("unable to send received data - {error}");
        break;
      }

      drop(frame_reader);
    }
  });

  Ok(response)
}

async fn run(arguments: CommandLineArguments) -> io::Result<()> {
  let dev = Device::with_path(&arguments.device)?;
  let format = dev.format()?;
  log::info!("Active format:\n{format:?}");

  let params = dev.params()?;
  log::info!("Active parameters:\n{params:?}");

  log::info!("Available formats:");
  for format in dev.enum_formats()? {
    log::debug!("{format:?}");

    if format.fourcc != v4l::format::FourCC::new(b"MJPG") {
      continue;
    }

    for framesize in dev.enum_framesizes(format.fourcc)? {
      for discrete in framesize.size.to_discrete() {
        log::debug!("    Size: {}", discrete);

        for frameinterval in dev.enum_frameintervals(framesize.fourcc, discrete.width, discrete.height)? {
          log::debug!("      Interval:  {}", frameinterval);
        }
      }
    }
  }

  if format.fourcc != v4l::FourCC::new(b"MJPG") {
    return Err(io::Error::new(io::ErrorKind::Other, "mjpg-format not supported"));
  }

  let last_frame_index = async_std::sync::Arc::new(async_std::sync::RwLock::new((None, Vec::with_capacity(1024))));
  let (switchbox_register, switchbox_receiver) = async_std::channel::unbounded();

  let mut server = tide::with_state(SharedState {
    last_frame: last_frame_index.clone(),
    switchbox: switchbox_register,
  });

  let reader_thread = async_std::task::spawn(async move {
    let frame_locker = last_frame_index.clone();
    let mut stream = v4l::prelude::MmapStream::with_buffers(&dev, v4l::buffer::Type::VideoCapture, 4).unwrap();
    let mut last_debug = std::time::Instant::now();
    let mut current_frames = 0;
    let mut listeners = vec![];

    loop {
      let before = std::time::Instant::now();
      let (buffer, meta) = stream.next()?;
      let after = std::time::Instant::now();
      current_frames += 1;
      let seconds_since = before.duration_since(last_debug).as_secs();
      let copied_buffer = buffer[0..(meta.bytesused as usize)].to_vec();

      // Take a writable lock our on our frame data and replace with the most recent data.
      let mut writable_frame = frame_locker.write().await;
      *writable_frame = (Some(std::time::Instant::now()), copied_buffer);
      drop(writable_frame);

      // See if we have any new web connections waiting to register their semaphore receivers.
      if let Ok(lisener) = switchbox_receiver.try_recv() {
        listeners.push(lisener);
      }

      // Iterate over any listener, sending our semaphore alone.
      if !listeners.is_empty() {
        let mut next = vec![];

        for listener in listeners.drain(0..) {
          if listener.is_closed() {
            continue;
          }

          if let Ok(_) = listener.send(()).await {
            next.push(listener);
          }
        }

        listeners = next;
      }

      if seconds_since > 3 {
        let frame_read_time = after.duration_since(before).as_millis();

        log::info!(
          "{current_frames}f ({seconds_since}s) {frame_read_time}ms | {}b",
          meta.bytesused
        );
        last_debug = before;
        current_frames = 0;
      }
    }
  });

  server.at("/stream").get(stream);
  server.at("/snapshot").get(snapshot);

  server
    .listen(&arguments.addr)
    .race(reader_thread)
    .await
    .map_err(|error| {
      log::info!("failed main threads - {error}");
      io::Error::new(io::ErrorKind::Other, error)
    })
    .map(|_| ())
}

fn main() -> io::Result<()> {
  env_logger::init();
  let arguments = CommandLineArguments::parse();
  async_std::task::block_on(run(arguments))
}
