# Codec components in the media worker image

The Scope media worker executable is distributed under Apache-2.0. The runtime
image also contains these version-pinned Debian codec packages:

- `ffmpeg` and `libavcodec59` 7:5.1.9-0+deb12u1. Debian builds FFmpeg with
  `--enable-gpl` and `--enable-libx264`; the resulting FFmpeg binaries are
  GPL-2.0-or-later according to the package copyright file.
- `libx264-164` 2:0.164.3095+gitbaee400-3, GPL-2.0-or-later.
- `libheif1` and `libheif-examples` 1.15.1-1+deb12u1. The libheif library is
  LGPL-3.0-or-later; its example tools have additional terms recorded by the
  package.

The installed packages retain their complete copyright and license records at:

- `/usr/share/doc/ffmpeg/copyright`
- `/usr/share/doc/libavcodec59/copyright`
- `/usr/share/doc/libx264-164/copyright`
- `/usr/share/doc/libheif1/copyright`
- `/usr/share/doc/libheif-examples/copyright`
- `/usr/share/common-licenses/`

Those package records cover additional codec libraries pulled into the image and
are authoritative for the shipped Debian binaries.
