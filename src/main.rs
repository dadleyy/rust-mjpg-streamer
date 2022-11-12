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
  last_frame: async_std::sync::Arc<async_std::sync::RwLock<(std::time::Instant, Vec<u8>)>>,
}

async fn render(request: tide::Request<SharedState>) -> tide::Result<tide::Response> {
  // Create the channel whose receiver will be used as a async reader.
  let (mut writer, drain) = async_std::channel::bounded(2);
  let buf_drain = futures::stream::TryStreamExt::into_async_read(drain);

  // Prepare the response with the correct header
  let response = tide::Response::builder(200)
    .content_type(format!("multipart/x-mixed-replace;boundary={BOUNDARY}").as_str())
    .body(tide::Body::from_reader(buf_drain, None))
    .build();

  // In a separate task, continously check our shared buffer's timestamp. If that value differs
  // from the timestamp of the last message sent on our end, send a new multipart chunk.
  async_std::task::spawn(async move {
    let mut last_frame = None;

    loop {
      let frame_reader = request.state().last_frame.read().await;
      let timestamp = match std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH) {
        Err(error) => {
          log::error!("unable to compute frame timestamp - {error}");
          break;
        }
        Ok(timestamp) => timestamp,
      };

      let timestamp = timestamp.as_millis();

      match last_frame {
        Some(other) if other == (*frame_reader).0 => continue,

        None | Some(_) => {
          // Start the buffer that we'll send using the boundary and some multi-part http header
          // context.
          let mut buffer = match last_frame.is_some() {
            // TODO: still figuring out whether or not this is necessary; strange behavior.
            false => format!(
              "\r\n--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Timestamp: {}\r\n\r\n",
              frame_reader.1.len(),
              timestamp,
            ),

            true => format!(
              "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Timestamp: {}\r\n\r\n",
              frame_reader.1.len(),
              timestamp,
            ),
          }
          .into_bytes();

          // Actually push the JPEG data into our buffer.
          buffer.extend_from_slice(frame_reader.1.as_slice());
          buffer.extend_from_slice(b"\r\n");

          last_frame = Some((*frame_reader).0);

          if let Err(error) = writer.send(Ok(buffer)).await {
            log::warn!("unable to send received data - {error}");
            break;
          }
        }
      }

      drop(frame_reader);
    }
  });

  Ok(response)
}

async fn run(arguments: CommandLineArguments) -> io::Result<()> {
  let dev = Device::with_path(&arguments.device)?;
  let format = dev.format()?;
  log::info!("Active format:\n{}", format);

  let params = dev.params()?;
  log::info!("Active parameters:\n{}", params);

  let mut found = false;

  log::info!("Available formats:");
  'outer: for format in dev.enum_formats()? {
    for framesize in dev.enum_framesizes(format.fourcc)? {
      for discrete in framesize.size.to_discrete() {
        if format.fourcc == v4l::format::FourCC::new(b"MJPG") {
          dev.set_format(&v4l::Format::new(
            discrete.width,
            discrete.height,
            v4l::format::FourCC::new(b"MJPG"),
          ))?;
          found = true;
          break 'outer;
        }
      }
    }
  }

  if !found {
    return Err(io::Error::new(io::ErrorKind::Other, "mjpg-format not supported"));
  }

  let last_frame_index = async_std::sync::Arc::new(async_std::sync::RwLock::new((
    std::time::Instant::now(),
    Vec::with_capacity(1024),
  )));

  let mut server = tide::with_state(SharedState {
    last_frame: last_frame_index.clone(),
  });

  let reader_thread = async_std::task::spawn(async move {
    let frame_locker = last_frame_index.clone();
    let mut stream = v4l::prelude::MmapStream::with_buffers(&dev, v4l::buffer::Type::VideoCapture, 4).unwrap();
    let mut last_debug = std::time::Instant::now();
    let mut current_frames = 0;

    loop {
      let before = std::time::Instant::now();
      let (buffer, _) = stream.next()?;
      let after = std::time::Instant::now();
      current_frames += 1;
      let seconds_since = before.duration_since(last_debug).as_secs();
      let mut writable_frame = frame_locker.write().await;
      *writable_frame = (std::time::Instant::now(), buffer.to_vec());

      if seconds_since > 3 {
        let frame_read_time = after.duration_since(before).as_millis();

        log::info!("{current_frames} frames (in {seconds_since} seconds) {frame_read_time}ms per");
        last_debug = before;
        current_frames = 0;
      }
    }
  });

  server.at("/image").get(render);

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
