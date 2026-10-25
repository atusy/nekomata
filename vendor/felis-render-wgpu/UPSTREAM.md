# Upstream provenance and embedding patch

Source: https://git.natsukium.com/felis-terminal/felis
Revision: 2a3030b237947497708b962ba6be8c2adade4951 (felis 0.3.1)
Original crate: crates/felis-render-wgpu (0.2.0)
License: Apache-2.0; upstream LICENSE is retained here.

The Rust modules, shaders and README are copied from that revision. Cargo.toml
resolves upstream workspace dependencies against the same pinned git source.
The application integration patch touches `src/lib.rs`, `src/capture.rs`, and
`src/shader.wgsl`:

- `new_embedded` accepts the host Device/Queue and shared FontStack, and uses staged uploads without
  enabling device features or requesting another adapter.
- The offscreen texture can be sampled, with an Rgba8Unorm compatible view.
- `embedded_texture_view` exposes that view for egui's native texture registry.
- Construction does not replace the host's device-lost callback.
- Embedded glyphs blend in encoded sRGB, matching egui's text composition.
  A separate glyph pass loads the same attachment through its non-sRGB view;
  the shader encodes glyph RGB once and leaves coverage/opacity untouched.
  Backgrounds, images and decorations retain upstream linear-light blending
  and ordering. The capture path uses the same glyph pass. Standalone surface
  and offscreen constructors retain upstream behavior.

The upstream renderer still owns shaping, glyph/image atlases, row invalidation,
instance buffers, decoration shaders and render passes. nekomata does not fork
those caches or rasterization policies. Text composition is an explicit local
policy at the embedding seam. There is no CPU frame capture on the interactive path.

On upstream updates, compare every vendored file to the pinned source, then
reapply and validate this seam. Remove the copy when an equivalent public
upstream embedding API becomes available.

Upstream benchmarks, proptest regression seeds and binary golden test fixtures
are not vendored. Dependency unit tests are not part of the application test
suite; see `docs/felis-rendering-reuse-audit.md` for the integration audit.
