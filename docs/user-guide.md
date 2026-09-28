# Pigment Prose user guide

Pigment Prose paints a landscape (a lake below mountains, a river valley, woods, rocks) in a watercolor-and-gouache style. The words you type choose the painting; the sliders shape how it is painted; you can export it as a PNG at up to 16384 pixels on a side.

This guide covers version 0.1 on Linux. It needs a hardware GPU with Vulkan (see [GPU and diagnostics](#gpu-and-diagnostics)).

## Your words are the randomness

Type or paste any text into **Prose**. The text is turned into a fingerprint (a SHA-256 digest), and that fingerprint seeds every random choice: the kind of landscape, where the mountains stand, the woods, the light, the wind on the water.

- **The words are not interpreted.** "A stormy sea" does not paint a storm. The text works like a seed, and changing one character gives a different painting.
- The same text, settings and shape always make the same composition. The **seed code** under the editor (8 hex digits) identifies the text a painting is made from.
- The painting updates about a third of a second after you stop typing.

## Controls

The controls are in three groups. Each slider shows its value, a notch at its default and what its two ends mean. **Reset** puts one slider back; **Reset all painting controls** puts all of them back.

| Group | Controls | What they do |
| --- | --- | --- |
| **Structure** | Form; under Advanced: Relief, Woodland density | Change the landscape itself: rounded or angular forms, how high and rugged it is, how much woodland. The composition morphs rather than jumping to a new one |
| **Paint handling** | Edge looseness, Wash / Gouache; under Advanced: Mark scale, Granulation, Paper grain | Change how it is painted. The landscape stays where it is |
| **Appearance** | Atmosphere, Color intensity, Palette | Haze with distance, how saturated the colors are, and the palette (Lakeshore or Golden evening) |

**Painting only** (Ctrl+\\) hides the controls so the painting fills the window.

## Compositions

**Another composition** (Alt+Right) keeps your words and every setting, and makes a new arrangement from them. **Previous** (Alt+Left) steps back. The number shown ("variation 3") is saved with the recipe, so you can return to a composition later.

**Shape** chooses the proportions: 16:9, 3:2, 1:1, 4:5 or 9:16. A new shape recomposes the painting (a portrait is not a crop of a landscape).

## Saving and opening recipes

A **recipe** (`.recipe.json`) is everything needed to make the painting again: the text's fingerprint, the composition number, the shape and every slider. It is small, and it is not the image.

- **Save** (Ctrl+S), **Save As…** (Ctrl+Shift+S) and **Open…** (Ctrl+O) use your desktop's file dialogs.
- If you have unsaved changes, opening another recipe or closing the window asks first: **Save…**, **Don't save** or **Cancel**.
- A recipe that cannot be read changes nothing and says which file failed. A save that fails keeps your work unsaved and leaves any existing file as it was.

### Your text stays private unless you choose otherwise

**By default, recipes do not contain your words**, only their fingerprint. Tick **Save the prose in the recipe file** to keep the words in the recipe (for example, to edit them later).

- A recipe saved without its words still makes exactly the same painting. When you open it, the editor is empty and says the words cannot be recovered from the file. Typing starts a new painting.
- The fingerprint cannot be turned back into your text. It is not encryption, though: someone who can guess your exact words can check the guess against the fingerprint. If the words themselves are sensitive, do not share the recipe.
- Your words never appear in exported images, file names the program suggests, or its messages.

## Exporting a PNG

**Export PNG…** (Ctrl+E) exports the painting as it is now: this composition and these settings.

- **Size is in pixels.** The presets show their real sizes for the current shape (for 16:9, 4K is 3840 × 2160 and 8K is 7680 × 4320). A custom width or height snaps to the painting's exact proportions. Sizes from 64 to 16384 pixels per side, up to 4:1, are allowed.
- **DPI is only a print label.** The painting's detail comes from its pixel count. To print 30 cm wide at 300 DPI you need about 3543 pixels across; choose that size rather than changing a DPI setting.
- Pressing **Export…** freezes the settings, then asks where to save. You can keep working while it exports; the export does not change.
- **Cancel export** stops at the next tile and removes the partial file. If you were replacing an existing file, it is kept unchanged.
- If an export fails (a full disk, a folder you cannot write to, not enough GPU memory, a GPU reset), the message says what happened and what to do. No partial file is left, and an existing file is unchanged.
- Closing the window during an export asks whether to keep exporting or to stop and close.
- **The PNG is only the image.** It has no watermark, no attribution, no text and no embedded recipe or prose. See [About the images you make](#about-the-images-you-make).

A preview is a smaller rendering of the same painting. An export at another size keeps the composition and the scale of brushwork and paper texture, but it is painted afresh at that size, not upscaled.

## Reproducibility and versions

- **On the same computer and version**, a recipe makes the same image every time, pixel for pixel.
- **On another GPU or driver** the same recipe makes the same composition, but pixels may differ very slightly.
- **Across versions:** each recipe records the generator and renderer versions that made it. This version is generator 2 and renderer 2. A recipe from another version opens with a note that it may compose or paint differently. The program ships one generator and one renderer and does not keep old ones. Saving updates the recorded versions.

Keep the exported PNG if you need that exact image forever; keep the recipe to make it again, or to change it.

## GPU and diagnostics

Pigment Prose paints on the GPU and **needs a hardware GPU with Vulkan**. It is tested on an NVIDIA GeForce RTX 4070 Ti (driver 615.71.09) and an Intel Raptor Lake-S integrated GPU, on CachyOS with KDE Plasma (Wayland). **CPU-only rendering is not currently supported or promised.**

- **The first start after installing, or after a graphics driver update, can be slow**, while the driver compiles and caches the painter's shaders. The window opens at once and says "Preparing the GPU painter". Measured on the test machine: about 4 seconds on the NVIDIA driver and about 35 seconds on Mesa's Intel driver, which also needs about 4 GB of memory while it compiles. Later starts take a fraction of a second.
- The status line under the painting names the GPU in use and how long the preview took.
- **Diagnostics** (Ctrl+D) lists the adapter, backend, driver, limits and preview counters. Include them when reporting a problem.
- If no suitable GPU is found, a window titled "Pigment Prose needs a hardware GPU" explains why, lists what was found and offers **Copy details**.
- If the GPU is reset (for example by a driver update), the preview stops and asks you to save your recipe and restart.
- On computers with two GPUs, `pigment-studio --adapter NAME` chooses the one that paints, for example `--adapter intel`.
- `pigment-studio --allow-software` accepts a software renderer. It is labelled everywhere as not GPU-accelerated, is much slower, and is unsupported.
- `pigment-prose gpu-info` lists the adapters from a terminal.

## Keyboard

Everything works from the keyboard. Tab moves between controls. On a slider: arrows ±0.01, Page Up/Down ±0.1, Home/End for the ends, Delete to reset. Ctrl+O/S/Shift+S/E/Q open, save, save as, export and quit. Ctrl+Plus and Ctrl+Minus zoom the interface.

## About the images you make

The program adds nothing to your images and asks for no credit. The fonts and code it is built from are listed in `THIRD-PARTY-NOTICES.md`; their licenses cover the software, and none of them places conditions on the pictures it makes. Exported paintings contain no fonts or third-party artwork: everything in them is generated.

This describes what the program does. It is not legal advice, and it cannot tell you what rights you have in text you type in (for example, someone else's writing) or how copyright treats generated images where you live.

## Command-line export

`pigment-prose export --recipe FILE.recipe.json --size 8k --out painting.png` exports a saved recipe without opening the window. `pigment-prose --help` lists everything else, most of which is for testing.
