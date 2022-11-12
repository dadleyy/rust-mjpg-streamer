use std::io;

use async_std::prelude::FutureExt;
use v4l::device::Device;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;

#[derive(Clone, Debug)]
struct SharedState {
  last_frame: async_std::sync::Arc<async_std::sync::RwLock<(std::time::Instant, Vec<u8>)>>,
}

async fn render(request: tide::Request<SharedState>) -> tide::Result<tide::Response> {
  log::info!("has request, sending sender");
  let (mut writer, drain) = futures::channel::mpsc::channel::<io::Result<Vec<u8>>>(1);
  let buf_drain = futures::stream::TryStreamExt::into_async_read(drain);

  let response = tide::Response::builder(200)
    .content_type("multipart/x-mixed-replace;boundary=boundarydonotcross")
    .body(tide::Body::from_reader(buf_drain, None))
    .build();

  async_std::task::spawn(async move {
    let frame_reader = request.state().last_frame.read().await;
    let mut last_frame = (*frame_reader).0.clone();
    drop(frame_reader);

    loop {
      let frame_reader = request.state().last_frame.read().await;
      if (*frame_reader).0 != last_frame {
        last_frame = (*frame_reader).0.clone();

        let mut buffer = format!(
          "--boundarydonotcross\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
          (*frame_reader).1.len()
        )
        .into_bytes();

        buffer.extend_from_slice((*frame_reader).1.as_slice());
        buffer.extend_from_slice(b"\r\n");

        if let Err(error) = writer.try_send(Ok(buffer)) {
          log::warn!("unable to send received data - {error}");
          break;
        }
      }
      drop(frame_reader);
    }
  });

  Ok(response)
}

async fn run(dev: Device) -> io::Result<()> {
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
            discrete.width,
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
    .listen("0.0.0.0:8080")
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

  let path = "/dev/video0";
  log::info!("Using device: {}\n", path);

  let dev = Device::with_path(path)?;

  async_std::task::block_on(run(dev))
}
