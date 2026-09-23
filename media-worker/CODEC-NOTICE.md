# Codec components in the media worker image

The Scope media worker executable is distributed under Apache-2.0. The runtime
image also contains these codec components:

- Debian `ffmpeg` and `libavcodec61` 7:7.1.5-0+deb13u1. Debian builds FFmpeg with
  `--enable-gpl` and `--enable-libx264`; the resulting FFmpeg binaries are
  GPL-2.0-or-later according to the package copyright file.
- Debian `libx264-164` 2:0.164.3108+git31e19f9-2+b1, GPL-2.0-or-later.
- Upstream `libheif` 1.23.5, built from a SHA-256-verified release archive.
  The library is LGPL-3.0-or-later; the example tools are MIT licensed.

The installed packages retain their complete copyright and license records at:

- `/usr/share/doc/ffmpeg/copyright`
- `/usr/share/doc/libavcodec61/copyright`
- `/usr/share/doc/libx264-164/copyright`
- `/scope/licenses/libheif-COPYING`
- `/scope/licenses/libheif-examples-COPYING`
- `/scope/licenses/libheif-1.23.5.tar.gz` (verified upstream source archive)
- `/scope/licenses/libheif-build.Dockerfile` (the build recipe)
- `/usr/share/common-licenses/`

The Debian package records cover additional codec libraries pulled into the
image. The copied upstream license files cover the locally built libheif tools.
The image workflow retains a CycloneDX SBOM with the libheif source hash and
queries OSV for advisories affecting that exact source release. Source-built
libheif is not identified by the Debian package scanner, so its OSV response
is retained separately; advisories without a published fix remain visible for
review.
