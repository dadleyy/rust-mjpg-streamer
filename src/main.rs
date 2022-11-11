use std::io;

use async_std::prelude::FutureExt;
use v4l::device::Device;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;

#[derive(Clone, Debug)]
struct SharedState {
  sender: async_std::channel::Sender<async_std::channel::Sender<Vec<u8>>>,
}

async fn render(request: tide::Request<SharedState>) -> tide::Result<tide::Response> {
  log::info!("has request, sending sender");
  let (sender, receiver) = async_std::channel::unbounded();
  request.state().sender.send(sender).await?;

  let (mut writer, drain) = futures::channel::mpsc::channel::<io::Result<Vec<u8>>>(1);
  let buf_drain = futures::stream::TryStreamExt::into_async_read(drain);

  let response = tide::Response::builder(200)
    .content_type("multipart/x-mixed-replace;boundary=boundarydonotcross")
    .body(tide::Body::from_reader(buf_drain, None))
    .build();

  async_std::task::spawn(async move {
    loop {
      match receiver.try_recv() {
        Err(error) if error.is_empty() => continue,
        Err(error) => {
          log::warn!("failed receiving - {error}");
          break;
        }
        Ok(data) => {
          let mut buffer = format!(
            "--boundarydonotcross\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
            data.len()
          )
          .into_bytes();
          buffer.extend_from_slice(data.as_slice());
          buffer.extend_from_slice(b"\r\n");

          if let Err(error) = writer.try_send(Ok(buffer)) {
            log::warn!("unable to send received data - {error}");
            break;
          }
        }
      };
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

  let (sender, receiver) = async_std::channel::bounded(2);

  let reader_thread = async_std::task::spawn(async move {
    let mut listeners: Vec<async_std::channel::Sender<Vec<u8>>> = vec![];
    let mut stream = v4l::prelude::MmapStream::with_buffers(&dev, v4l::buffer::Type::VideoCapture, 4).unwrap();
    let mut last_debug = std::time::Instant::now();
    let mut current_frames = 0;

    loop {
      let before = std::time::Instant::now();
      let (buffer, _) = stream.next()?;
      let after = std::time::Instant::now();
      current_frames += 1;
      let seconds_since = before.duration_since(last_debug).as_secs();

      if seconds_since > 3 {
        let frame_read_time = after.duration_since(before).as_millis();

        log::info!("{current_frames} frames (in {seconds_since} seconds) {frame_read_time}ms per");
        last_debug = before;
        current_frames = 0;
      }

      match receiver.try_recv() {
        Err(error) if error.is_empty() => (),
        Err(error) => {
          log::warn!("unable to receive incoming listeners on reader thread - {error}");
          break;
        }
        Ok(inner) => {
          log::info!("has new listener, yay");
          listeners.push(inner);
        }
      }

      if !listeners.is_empty() {
        let drained = listeners
          .drain(0..)
          .collect::<Vec<async_std::channel::Sender<Vec<u8>>>>();

        for listener in drained.into_iter() {
          match listener.send(buffer.to_vec()).await {
            Ok(_) => listeners.push(listener),
            Err(error) => {
              log::warn!("unable to send - {error}");
            }
          }
        }
      }
    }

    Ok(())
  });

  let mut server = tide::with_state(SharedState { sender });

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
