# Rusty-MJPG

This application is meant to be two things:

1. A standalone, rust-alternative to [mjpg-streamer], a popular webcam -> http stream application used with
   octoprint for 3d printing hobbyists.
2. An example of how one would go about streaming mjpeg data from a webcam while using the [tide] rust http framework.


### Attribution

The majority of this functionality is simply combining the efforts of the [async-rs] and [v4l] crates into a single
application; large thank you to all of the folks working on those projects!

[mjpg-streamer]: https://github.com/jacksonliam/mjpg-streamer
[tide]: https://github.com/http-rs/tide
[async-rs]: https://github.com/async-rs
[v4l]: https://github.com/raymanfx/libv4l-rs
