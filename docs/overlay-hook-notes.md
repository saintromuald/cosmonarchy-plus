# Overlay hook notes (next chunk: saturation numbers)

Read-only research so the overlay work starts from facts, not guesses. All references are
to `reference/cosmonarchy-gptp` (the C++ plugin we cannot build locally) and the live
1.16.1 exe; the Plus plugin is Rust and must reach the same primitives by address.

## Where the mod draws its own overlays

- `hooks/game_hooks.cpp:385 draw_custom_ui()` — per-tick overlay pass: aura circles,
  ability ranges, rally lines. It calls `g_graphics.draw_circle(...)` /
  `g_graphics.draw_text(x, y, str, font, ON_MAP)`.
- `graphics/graphics.h:16` — `draw_text(s32 x, s32 y, const std::string&, FontSize,
  CoordType)`. `CoordType::ON_MAP` (graphics/Shape.h:37) means the shape is scaled with
  terrain zoom and converted from world to screen by the draw hook.
- `graphics/draw_hook.cpp` — hooks the game's draw proc (`g_old_draw_game_proc`), draws
  the world, then `g_graphics.draw_world_shapes()`; screen-space UI is deferred separately.
  **This is the correct place for a map overlay:** it is a draw pass, not the simulation,
  so it never runs during a rollback re-sim and is safe to add.
- `graphics/graphics.cpp` — shapes are queued, not drawn inline; a text shape only
  becomes pixels in the draw hook.

## Low-level text primitives (callable from Rust by address)

| Item | Address / source |
|---|---|
| `helper::bw_print_xy(const char*, s32 x, s32 y)` | `0x004202B0` (`BW/bwhelper.h:1423`) |
| `helper::set_font_handle(u32* font)` | `0x0041FB30` (`bwhelper.h:950`) |
| `helper::get_color_handle(TextColor)` | `0x0041F610` (`bwhelper.h:949`) |
| `font_base` (array of `graphics::Font*`) | `0x006CE0F4` (`bwdata.h:934`); in-game font is `font_base[1]` |
| `g_draw_text` (Box16: left/top/right/bottom) | `0x006CE0C8` (`bwdata.h:979`) |
| `g_print_xy_alignment` (u8) | `0x006CE110` (`bwdata.h:1319`) |

Existing usage pattern (`hooks/display/screen_text.cpp:17-25`):

```cpp
helper::set_font_handle((u32*)font_base[1]);
helper::get_color_handle(color);
g_draw_text.left = left; g_draw_text.top = top;
g_draw_text.right = right; g_draw_text.bottom = top + g_text_area_y;
g_print_xy_alignment = alignment;
helper::bw_print_xy(text, left, top);
helper::set_font_handle(nullptr);
```

## Coordinates

- `ON_MAP` shapes are world coordinates; the draw hook applies the camera/zoom transform.
  If Plus draws via `bw_print_xy` directly it must do that transform itself:
  screen = (world - camera) * zoom / 10000 + origin. Confirm against
  `systems/display/zoom.h` (`WorldZoom`) and `bwdata.h` (`g_screen_x`, `0x00628448`) before
  committing. Safer first cut: hook the draw proc where the world is already at 1:1 and
  call `bw_print_xy` with the same transform the shapes use.
- Text above a mineral field: node position is the patch centre (`Unit.position`, 32
  pixels per tile); offset up by the sprite's selection-circle offset or ~20 px.

## Colours

- Text colours are `TextColor` (`BW/enumerations.h:199`), passed to `get_color_handle`.
- Race colours already exist: `get_race_textcolor(RaceId)` (`BW/enumerations.h:322`):
  Zerg `Purple`, Terran `RiverBlue`, Protoss `Yellow`, Askosi `DarkGreen`,
  Bloodsummon `Red`, Kogri `Grey2`, default `White`.
- Desired behaviour: **white** until the patch is fully saturated, then the *owner's*
  race colour. Read the race from `g_player_table[player].race` (owner of the node's
  depot/player), not from the local player, so allies' patches colour correctly.

## What the overlay must read (no writes)

- Per resource node: `Unit.id` (mineral field / geyser classes), `Unit.player`,
  `Unit.position`, `Unit.building.resource.resource_amount`.
- Assigned workers: `UnitExt.values[ValueAssignedHarvesters]` **if** the Rust side can
  resolve `Unit -> UnitExt` safely (`data_ext.h`); otherwise count units whose harvest
  order targets the node (`Unit.order_target` / `main_order_id == OrderId::Harvest*`,
  stable in the snapped state). Counting is O(units) and only for visible nodes.
- Worker tier: unit id → tier table (ids from `hooks/economic_units.cpp`), and speed from
  the live units/flingy data.

## Open questions for the overlay task

1. Which draw-proc pointer to hook from Rust, and whether `g_old_draw_game_proc` is a
   stable exported/global address in 1.16.1 (it is set by storm at present time).
2. Whether `bw_print_xy` needs the full `g_draw_text` box set or only left/top for a
   short label (the HP-bar path draws without it; the message log sets it).
3. Zoom transform: reuse the mod's `WorldZoom` state or compute from `g_screen_x/y` and
   the zoom percentage.
4. Text width for centring: use alignment `0x12`/`0x13` (right/center) as the message log
   does, instead of measuring the string.
