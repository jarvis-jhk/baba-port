# `Assets.dat`

Every build of the game (Linux, Windows, macOS) is a Chowdren translation
of the original Clickteam Fusion project. Everything Fusion itself held
(as opposed to the plain files under `Data/`) is packed into `Assets.dat`
next to `Data/`: the engine-side images, the sound effects and the
shaders. Nothing in it is named; the executable refers to assets by
index. Reader: `baba_formats::assets::AssetsDat`.

## Builds

| Build | Where | Size (Steam, 2026-10) | Shaders |
|---|---|---|---|
| Linux | `Assets.dat` next to `Data/` and `bin64/Chowdren` | 21 806 432 | GLSL source |
| macOS | `Baba Is You.app/Contents/Resources/Assets.dat` | 21 806 432 (same bytes as Linux) | GLSL source |
| Windows | `Assets.dat` next to `Data/` and `Baba Is You.exe` | 21 786 080 | Direct3D 9 bytecode |

The container, the header (same `N`, `K`, entry order), the image region
and the sound region are byte-identical in all three; only the shader
entries (and so the shader region size and the offsets of the entries
after the first shader) differ. Image and sound indices therefore mean
the same thing in every build. `Data/` is identical too.

## Container

All integers little endian.

```text
header:
  N x { u32 offset, u32 size }     one entry per asset, absolute file offsets
  K x   u32 region size            total bytes of each asset kind, in kind order
data:
  region 0 (images) | region 1 (sounds) | region 2 (fonts) | region 3 (shaders) | region 4 (files) | region 5
```

- The header length equals the offset of the first asset (the first u32 of
  the file), so `8 N + 4 K = header length`.
- The region sizes add up to `file length - header length`. The reader finds
  `K` by trying small values until both equations hold (in the shipped file
  `K = 6`, `N = 3571`, header 28 592 bytes).
- Entries are stored in kind order; the kind of an entry is the region its
  bytes fall in. Each asset starts 16-byte aligned (zero padding between).
- The shipped file: 3206 images (3.6 MB), 245 sounds (18 MB), no fonts, 120
  shaders (0.17 MB), no files, empty sixth region. The kind names come from
  the content of each region; the meaning of the empty regions is a guess.

## Image entries

```text
u16 width
u16 height
i16 hotspot x, i16 hotspot y          origin used when drawing
i16 action point x, i16 action point y
u8  flags                              0 normal; 1 seen on some solid images; 2 = no pixel data
u32 stream size                        bytes that follow (0 for empty images)
stream: repeated until consumed
  u32 compressed size
  ... LZ4 block (raw block format, no frame header)
```

The concatenated output of the blocks is `width * height * 4` bytes of
straight-alpha RGBA, rows top to bottom. Each block decompresses to at most
64 KiB. Flag 2 entries are 0x0 placeholders with an empty stream.

Contents (from a contact sheet of all of them): the Fusion-side graphics:
menu and UI elements, the glyphs of the in-game text font (Latin with
accents, Cyrillic, ...), button and control prompts (some in Finnish), logos
for the credits, particle and effect images, ending/extra artwork, copies of
object sprites used by menus and cutscenes, and full-screen plates (up to
792x432). Colours confirm the RGBA byte order.

## Sound entries

```text
u32 format        2 in every entry
u32 size
... file bytes    an Ogg Vorbis file ("OggS")
```

These are the game's sound effects (the `Data/Sounds/` directory only holds
an example file). Their names exist only inside the executable's string
table, so the index-to-name mapping is not recoverable from `Assets.dat`
alone. In every build's executable they form a table of string pointers
(a hash table with empty slots) next to a parallel array of 32-bit sound
indices. The pointers are plain virtual addresses in all three: the ELF
is not position-independent, the PE stores them for its preferred image
base (relocations fix them up only if it loads elsewhere), and the
Mach-O uses classic rebase opcodes (`LC_DYLD_INFO_ONLY`, no chained
fixups), so the file holds the link-time address. baba-port maps
addresses to file offsets with the container's own tables (ELF program
headers, PE section headers + image base, Mach-O `LC_SEGMENT_64`; a
universal Mach-O is tried slice by slice), finds the table from a
pointer to a known name, and reads the names from the player's own
executable at run time (`baba_formats::exe`, used by
`crates/game/src/sounds.rs`). All three builds give the same 245 names
in the same order.

| Build | Executable | Container |
|---|---|---|
| Linux | `bin64/Chowdren` | ELF64 x86-64 |
| Windows | `Baba Is You.exe` | PE32+ x86-64 (image base `0x140000000`) |
| macOS | `Baba Is You.app/Contents/MacOS/Chowdren` | Mach-O 64-bit x86-64 (thin) |

An imported or hand-copied install may keep the executable elsewhere:
`baba_formats::GameInstall::executables` also looks at `Chowdren` and
`MacOS/Chowdren` next to `Data/` and at `../MacOS/Chowdren` from a
bundle's `Contents/Resources`.

## Shader entries

```text
u32 vertex program length
... vertex program
u32 fragment program length
... fragment program            (the lengths add up to the entry size - 8)
```

The Fusion effect shaders (colour blends, distortions, etc.) as translated
by Chowdren, 120 entries in every build:

- Linux and macOS: GLSL source text (`#version 120`, CRLF line ends in
  most entries).
- Windows: compiled Direct3D 9 shader model 3 bytecode, `vs_3_0`
  (version token `0xFFFE0300`) and `ps_3_0` (`0xFFFF0300`), each with a
  `CTAB` constant table comment ("Microsoft (R) HLSL Shader Compiler
  10.1") and the `0x0000FFFF` end token. The game runs them through
  `dxvk.dll`.

`AssetsDat::shader_language` tells the two files apart (the vertex part
of the first shader starts with a D3D9 vertex version token or not). The
port uses its own shaders; these are only reference.

## Not in `Assets.dat`

Levels, object definitions, sprites, palettes, music, languages and the
level background images are plain files under `Data/` (see the other
documents). The port needs `Assets.dat` (and the executable, for the
names) for the sound effects and the engine-side images; without them it
plays silently and draws its own stand-ins.
