# Vendored packages

`pagent-v0.1.0-47b796c.tgz` is the private Pagent SDK and CLI package built from
Pagent commit `47b796c`. Its SHA-256 digest is
`62792ab450d5a2d8dd29f474b29b705e069c2a120e79e2c807f138657001f618`.

Scope installs the archive from this directory so CI and Railway do not need
credentials for the private Pagent repository. Replace the archive only with a
package produced by Pagent's release builder, then update the commit and digest
above.

Pagent is separately licensed under Apache-2.0. Its original release builder
omitted the license from this archive. The upstream grant from commit
`d0e8ba60e39e058d189d7dd121740afc30f32900` is preserved in
[`legal/upstream/pagent-d0e8ba60e39e-LICENSE`](../../legal/upstream/pagent-d0e8ba60e39e-LICENSE)
and the generated web notices. Only licensing and repository instructions changed
between that revision and the bundled revision; application code is unchanged.
The supplement is bound to the archive checksum above. A replacement archive
must include the upstream license and update the inventory.
