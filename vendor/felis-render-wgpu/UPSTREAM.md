# Upstream provenance and embedding patch

Source: https://git.natsukium.com/felis-terminal/felis
Revision: 2a3030b237947497708b962ba6be8c2adade4951 (felis 0.3.1)
Original crate: crates/felis-render-wgpu (0.2.0)
License: Apache-2.0; upstream LICENSE is retained here.

The Rust modules, shaders and README are copied from that revision. Cargo.toml
resolves upstream workspace dependencies against the same pinned git source.
Only src/lib.rs has an application integration patch:

- `new_embedded` accepts the host Device/Queue and uses staged uploads, without
  enabling device features or requesting another adapter.
- The offscreen texture can be sampled, with an Rgba8Unorm compatible view.
- `embedded_texture_view` exposes that view for egui's native texture registry.
- Construction does not replace the host's device-lost callback.

The upstream renderer still owns shaping, glyph/image atlases, row invalidation,
instance buffers, decoration shaders and render passes. nekomata does not fork
those policies. There is no CPU frame capture on the interactive path.

On upstream updates, compare every vendored file to the pinned source, then
reapply and validate this seam. Remove the copy when an equivalent public
upstream embedding API becomes available.
