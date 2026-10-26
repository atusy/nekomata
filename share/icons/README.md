# nekomata icons

`nekomata.svg` is the original cat artwork. The application PNG and macOS ICNS
add a pale rounded background; `palette-button.png` crops the whitespace and
uses a white silhouette so egui can tint it to match the command palette
button's state.

Regenerate the committed raster assets from the SVG:

```sh
nix develop -c python3 scripts/generate-icons.py
```

The runtime embeds the PNG files and caches the palette button texture. The Nix package
also installs the desktop icons and the macOS bundle's ICNS resource.
