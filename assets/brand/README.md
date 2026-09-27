# Oxyn brand

> **Authority**: the geometry of the symbol, the color tokens and the file to
> use depending on the context. Every Oxyn visual goes through this document;
> no symbol is redrawn by hand.

## The symbol

**Pair — one mass, one cut.** A solid block with continuous curvature, notched
by a single vertical saw cut that does not go through: the mass stays **one**
piece, and the fragment to the right of the notch carries the accent. The user
and the agent, same material, side by side — what
[VISION](../../docs/VISION.md) promises: agents work *alongside* the user,
never in their place.

The cut never goes through. A through slot produces two juxtaposed panels,
that is the system glyph `sidebar.trailing` ("show inspector" button, Split
View): at 32 and 16 px, the drawing is the same. The bridge of material under
the notch is what distinguishes the mark from an interface control.

## Geometry

Canvas 1024, front view, no rotation.

| Parameter | Value |
|---|---|
| Mass | 672 centered (176–848), i.e. 65.6% of the side |
| Outer radius | 168, G1 blend on the corners |
| Slot | width 64, at x 576–640 |
| Cut | from the top, stopping at y 704 (79% of the height) |
| Slot bottom | half-circle r 32 |
| Mouth fillets | 44 |
| Bridge | 144 (21% of the height) |
| Pieces | 400 / 208, i.e. 1.92 : 1 |

The slot is aligned on the 64 grid to fall on a whole column:
1 px at 16 px, 2 px at 32 px, 4 px at 64 px. **Do not move the slot**: any
offset recreates gray columns below 64 px.

Minimum material 144, slot 64, all radii ≥ 32 — above the thresholds macOS 26
requires for layered rendering.

## Colors

| Token | Value | Use |
|---|---|---|
| Slate | `#172E33` → `#0D2125` | tile, light mode; mark on a light background |
| Deep slate | `#102428` → `#061519` | tile, dark mode |
| Frost | `#E8F3F2` → `#D6E7E5` | glyph on a dark tile; tile B |
| Patina | `#007467` → `#005B53` | tile of palette D |
| Deep patina | `#005B53` | glyph on a light tile |
| **Accent** | **`#1C8D7A`** | the fragment, everywhere — verdigris |

A single accent for the whole system. It holds the 3:1 contrast ratio on every
tile: 3.49 on the top of the light slate, 4.07 at the bottom, 3.96 and 4.56 in
dark, 3.18 on the bottom of the frost tile, and 3.58 against the frost glyph,
which keeps the two pieces distinct.

The palette is the interface's (`apps/desktop/src/styles.css`): the accent is
the verdigris of actions — the patina of copper —, the slate that of dark
surfaces. It replaces, since 2026-09-23, the graphite, bone and orange oxide
(`#BF4C22`) of the first version; only the colors changed, the paths stayed
identical to the byte.

The accent before last, `#9A3B1E`, only held 2.22:1 on the light tile, the
most common mode: the pair could not be read where the icon is seen most.

## Which file to use

| Context | File |
|---|---|
| macOS application icon | `Oxyn.icns`, or `svg/oxyn-appicon-light.svg` |
| Dock, App Store, showcase | `svg/oxyn-appicon-D.svg` — mono, a single material |
| macOS 26 Tinted mode | `svg/oxyn-appicon-tinted.svg` |
| 16 and 32 px | `svg/oxyn-appicon-{16,32}.svg` — redrawn on the pixel grid |
| On a light background | `svg/oxyn-mark.svg` |
| On a dark background | `svg/oxyn-mark-white.svg` |
| Website, documentation | `svg/oxyn-mark-accent.svg` |
| Favicon | `favicon.svg` |

The 16 and 32 px sizes are **not** reductions of the 1024: they are redrawn on
the pixel grid. A reduction of the 1024 leaves a one-pixel gray fringe on the
edge of the mass.

## What is never done

The geometry does not change: the `<path id="glyph">` is identical, to the
byte, in the eight 1024 files; only the colors vary. The symbol is never
tilted, distorted, or placed on a plate inside the tile. No effect is baked
into the path — shadow, bevel, blur: macOS 26 provides them per layer.

## Regenerating

The renders derive from the SVGs of `svg/`. Any rework starts from there,
never from a PNG.

```bash
rsvg-convert -w 1024 -h 1024 svg/oxyn-appicon-light.svg -o png/oxyn-appicon-light-1024.png
iconutil -c icns Oxyn.iconset -o Oxyn.icns
```

`oxyn-sheet-corrige.png` is the control sheet: the five tiles, the three
marks, the actual sizes and the two Dock strips.
