#![forbid(unsafe_code)]

use std::io;

use clap::Parser;

use async_std::prelude::FutureExt;
use v4l::device::Device;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;

const BOUNDARY: &str = "mjpg-boundary";

/// Source:
/// https://github.com/jacksonliam/mjpg-streamer/blob/310b29f4a94c46652b20c4b7b6e5cf24e532af39/mjpg-streamer-experimental/plugins/input_uvc/huffman.h#L26-L62
const HUFFMAN: [u8; 420] = [
  0xff, 0xc4, 0x01, 0xa2, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
  0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x01, 0x00, 0x03, 0x01, 0x01,
  0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
  0x07, 0x08, 0x09, 0x0a, 0x0b, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00,
  0x00, 0x01, 0x7d, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
  0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62,
  0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37,
  0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a,
  0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85,
  0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6,
  0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
  0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7,
  0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0x11, 0x00, 0x02, 0x01, 0x02, 0x04,
  0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21,
  0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1,
  0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19,
  0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
  0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74,
  0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95,
  0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6,
  0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7,
  0xd8, 0xd9, 0xda, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
  0xf9, 0xfa,
];

#[derive(Parser, Debug)]
#[command(author, version = option_env!("RUSTY_MJPG_VERSION").unwrap_or_else(|| "dev"), about, long_about = None)]
struct CommandLineArguments {
  #[arg(short = 'd', long)]
  device: String,

  #[arg(short = 'a', long)]
  addr: String,

  #[arg(
    short = 'e',
    long,
    help = r#"Encode the image with a default huffman table. This may be required for the jpeg output of some webcams.
    To determine whether or not your camera needs this, run with `--features validation`, toggling this value.
    "#
  )]
  huffman: bool,
}

#[derive(Clone, Debug)]
struct SharedState {
  last_frame: async_std::sync::Arc<async_std::sync::RwLock<(Option<std::time::Instant>, Vec<u8>)>>,
  switchbox: async_std::channel::Sender<async_std::channel::Sender<()>>,
}

async fn snapshot(request: tide::Request<SharedState>) -> tide::Result<tide::Response> {
  let frame_reader = request.state().last_frame.read().await;

  let response = tide::Response::builder(200)
    .header("Access-Control-Allow-Origin", "*")
    .content_type("image/jpeg")
    .body(frame_reader.1.clone())
    .build();

  drop(frame_reader);

  Ok(response)
}

async fn access(_request: tide::Request<SharedState>) -> tide::Result<tide::Response> {
  Ok(
    tide::Response::builder(200)
      .header("Access-Control-Allow-Origin", "*")
      .body("")
      .build(),
  )
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
    .header("Access-Control-Allow-Origin", "*")
    .content_type(format!("multipart/x-mixed-replace;boundary={BOUNDARY}").as_str())
    .body(tide::Body::from_reader(buf_drain, None))
    .build();

  // In a separate task, continously check our shared buffer's timestamp. If that value differs
  // from the timestamp of the last message sent on our end, send a new multipart chunk.
  async_std::task::spawn(async move {
    let mut last_frame = None;
    let mut frame_count = 0;
    let mut last_debug = std::time::Instant::now();

    loop {
      // Await for our semaphore that will let us know a frame is available.
      if let Err(error) = receiver.recv().await {
        log::warn!("unable to pull semaphore -  {error}");
        break;
      }

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

      // Start the buffer that we'll send using the boundary and some multi-part http header
      // context.
      let mut buffer = format!(
        "--{BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        frame_reader.1.len(),
      )
      .into_bytes();

      // Actually push the JPEG data into our buffer.
      buffer.extend_from_slice(frame_reader.1.as_slice());

      // Update this thread's reference to the last frame sent.
      last_frame = (*frame_reader).0;
      drop(frame_reader);

      let now = std::time::Instant::now();
      frame_count += 1;

      if now.duration_since(last_debug).as_secs() > 3 {
        log::info!("{frame_count} frames in 3 seconds");
        last_debug = now;
        frame_count = 0;
      }

      if let Err(error) = writer.send(Ok(buffer)).await {
        log::warn!("unable to send received data - {error}");
        break;
      }
    }
  });

  Ok(response)
}

async fn run(arguments: CommandLineArguments) -> io::Result<()> {
  let mjpg = v4l::FourCC::new(b"MJPG");
  let dev = Device::with_path(&arguments.device)?;

  let caps = dev.query_caps()?;
  log::info!("caps: {caps:?}");

  let ctrls = dev.query_controls()?;
  log::info!("ctrls: {ctrls:?}");

  let mut format = dev.format()?;
  log::info!("active format: ({:?}) {format:?}", format.fourcc.str());

  let params = dev.params()?;
  log::info!("active parameters: {params:?}");

  for desc in dev.enum_formats()? {
    if desc.fourcc != mjpg {
      log::info!("found non-jpeg format: {:?}", desc.fourcc.str());
      continue;
    }

    for framesize in dev.enum_framesizes(desc.fourcc)? {
      for discrete in framesize.size.to_discrete() {
        log::info!("found mjpg framesize - {}x{}", discrete.width, discrete.height);

        if discrete.width < format.width && discrete.height < format.height {
          let f = v4l::Format::new(discrete.width, discrete.height, mjpg);
          dev.set_format(&f)?;
          format = dev.format()?;
        }
      }
    }
  }

  if format.fourcc != mjpg {
    return Err(io::Error::new(io::ErrorKind::Other, "mjpg-format not supported"));
  }

  log::info!("final format: ({:?}) {format:?}", format.fourcc.str());

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
    let mut had_huffman = false;

    loop {
      let before = std::time::Instant::now();
      let (buffer, meta) = stream.next()?;

      let normalized = if arguments.huffman && !had_huffman {
        // Not all webcams will include the huffman table in their mjpg frame buffer; to support as
        // many browsers as possible, make sure here we have that data inside our buffer.
        let mut normalized = Vec::with_capacity(buffer.len());
        let mut i = 0;

        // 2048 appears in the original mjpg-streamer as the maximum amount of bytes to look at.
        while i < 2048 {
          if buffer[i] == 0xff && buffer[i + 1] == 0xC4 {
            log::info!("found huffman in raw camera payload");
            had_huffman = true;
            break;
          }

          // If we're at the start of "start of frame" marker, toss our default huffman table into
          // the buffer.
          if buffer[i] == 0xff && buffer[i + 1] == 0xC0 {
            normalized.extend_from_slice(&HUFFMAN);
            break;
          }

          normalized.push(buffer[i]);
          i += 1;
        }

        // Copy the remainder of our buffer into the normalized data.
        normalized.extend_from_slice(&buffer[i..meta.bytesused as usize]);

        normalized
      } else {
        buffer.to_vec()
      };

      #[cfg(feature = "validation")]
      {
        // Validate that the jpeg buffer is valid.
        let mut dec = jpeg_decoder::Decoder::new(normalized.as_slice());
        if let Err(error) = dec.decode() {
          log::warn!("invalid jpeg frame received - {error}");
          async_std::task::sleep(std::time::Duration::from_millis(100)).await;
          continue;
        }
        log::debug!("validated frame");
      }

      // Take a writable lock our on our frame data and replace with the most recent data.
      let mut writable_frame = frame_locker.write().await;
      *writable_frame = (Some(std::time::Instant::now()), normalized);
      drop(writable_frame);

      // Grab the current time; this measures how long it took for us to take the capture and add
      // our huffman data, if necessary.
      let after = std::time::Instant::now();
      current_frames += 1;
      let seconds_since = before.duration_since(last_debug).as_secs();

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

          // Keep this semaphore channel around if we were able to send.
          if listener.send(()).await.is_ok() {
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

  server.at("/*").head(access);
  server.at("/*").options(access);
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
