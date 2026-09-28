# Cake: design

A real-time strategy game on a ring. Up to eight players share a thin band
around a circle, each starting in an equal sector with an immovable HQ. Every
player has exactly two neighbours, so every player always has two fronts.

The game is meant to produce a particular kind of tension:

- defending one side while pushing on the other;
- scouting what everyone else is doing, cheaply and imperfectly;
- impromptu, **purely social** diplomacy: "you hit him, I'll leave you alone",
  "let me through", and the betrayals that follow.

There are no treaty mechanics. Everything is always hostile; deals are talk.

## Pillars

1. **The ring is the map.** Its geometry (two neighbours, a thin band, finite
   room for economy) generates the strategy. No terrain, no chokepoints beyond
   the band itself.
2. **Information is a resource.** Radar says *something* is there; only a real
   scout says *what*. Bluffing is possible.
3. **Readable at a glance.** Everything is a coloured polyline. The whole ring
   fits on screen.

## Geometry

The map is an annulus centred on the origin: inner radius **400**, outer
radius **500**. With N players, seat `i` owns the sector centred at angle
`i·τ/N`, and its HQ sits there at radius 450.

| | Inner edge | Midline | Outer edge |
|---|---|---|---|
| Circumference | ~2513 | ~2827 | ~3142 |

At N = 6, HQs are about 471 units apart along the midline.

**Neighbours.** Reaching a non-neighbour means marching through someone's
territory. That gives "let me pass" and "hit him, not me" deals real stakes.
When a player is eliminated the ring closes, and their two neighbours now
border each other.

**Band width vs. weapon range.** The band is 100 wide, and that makes turret
range a key number:

- turret range is 55, measured to the target's edge. A turret at mid-band
  (r = 450) just reaches both edges of the band, but only across a gate about
  46 units long;
- a raider hugging the edge is under fire for well under a second, so it
  can slip past a single turret;
- two or three turrets, or a screen of brawlers, seal a front.

This is deliberate: raids should be possible but answerable.

**Economy scarcity.** Economy buildings keep at least **150** from every other
economy building (anyone's) and from every HQ.

- At N = 6 that leaves about two safe sites per player (at their HQ ± 150)
  and about one **contested site at each border**.
- So growing your economy means building toward your neighbours, and border
  land is worth fighting over.
- Staggering buildings radially packs a few more in. That is a knob to tune.

## Economy

- One resource, **supply**. No harvesting.
- Income: the HQ gives **5/s**, each economy building adds **2/s**.
- Start: **100** supply and one utility.
- The HQ has one production queue of up to **5**; supply is paid on queueing
  and refunded on cancel. Units walk to the rally point when they spawn.
- Unit cap: **60** per player.

## Units

The counter triangle: **brawlers beat raiders, raiders beat skirmishers,
skirmishers beat brawlers.** Raiders also do triple damage to structures and
utilities, which is what makes them raiders.

| Unit | Cost | Build | HP | Speed | Range | Damage / cooldown | Vision |
|---|---|---|---|---|---|---|---|
| Brawler | 60 | 6 s | 240 | 28 | 10 (melee) | 7 / 0.5 s (14 dps) | 70 |
| Skirmisher | 50 | 5 s | 90 | 32 | 65 | 10 / 1 s (10 dps) | 90 |
| Raider | 45 | 4 s | 70 | 65 | 18 | 5 / 0.5 s (10 dps) | 100 |
| Utility | 50 | 6 s | 80 | 30 | – | – | 80, radar 220 |

Damage multipliers:

| Attacker → target | Multiplier |
|---|---|
| Brawler → raider | ×2 |
| Skirmisher → brawler | ×3 |
| Raider → skirmisher | ×1.5 |
| Raider → HQ, economy, turret, utility | ×3 |

Ranges are measured from the attacker's centre to the target's edge.

What those numbers mean in practice:

- a raider crosses half the ring (~1414) in about 22 s; a brawler takes
  about 50 s;
- at N = 6 a raider reaches a neighbour's HQ in about 7 s, a brawler in 17 s;
- the rules tests fight equal-cost armies (360 supply) at a neutral point, and
  each counter wins outright with no micro.

### Utility

- **Radar** (220): enemies inside it but outside anyone's vision show as blips.
- **Build turret:** costs 75. The utility walks to the site, lays a frame (10%
  HP), and builds it over 8 s. A frame does not shoot. Other utilities can
  help by repairing it.
- **Repair:** 15 HP/s to a friendly entity within 30, free.
- **Deploy:** walk to a site and spend 5 s becoming an **economy building**.
  The utility is consumed. The site is checked when deploying starts and again
  when it ends.

### Structures

| Structure | HP | Weapon | Vision | Notes |
|---|---|---|---|---|
| HQ | 2000 | range 60, 10 dps | 120 | Fends off a raider or two, not an army |
| Turret | 500 | range 55, 15 dps | 70 | Built by utilities |
| Economy | 400 | – | 50 | +2 supply/s |

No structure may be placed within 100 of an enemy HQ.

## Information

- **Vision** shows everything about an entity: kind and HP.
- **Radar** shows a blip: a position, nothing more.
- Vision, not radar, is required to target something.

Because the simulation is lockstep, fog is enforced **on screen only**: every
peer holds the whole state. That is acceptable among friends, and it is a
known trade-off (see Risks).

## Winning

A player is eliminated when their HQ falls. Their units and structures are
removed at once, and the ring closes around the gap. The last HQ standing
wins. Target match length: 15–25 minutes. For calibration: a four-bot match
currently ends after about 14½ minutes, and six bots are all still standing
at five minutes.

## Controls

Classic RTS micro:

| Input | Effect |
|---|---|
| Left click / drag | Select (Shift adds) |
| Right click | Move; attack an enemy; utilities repair a friendly; HQ alone: set rally |
| A, then click | Attack-move |
| S | Stop |
| B, then click | Utility builds a turret |
| D, then click | Utility deploys into an economy building |
| Q W E R | Queue brawler, skirmisher, raider, utility |
| X | Cancel the last queued unit |
| Ctrl+A / Space | Select the army / the HQ |
| Wheel, middle-drag, arrows, H | Zoom, pan, home view |

The view is rotated so your HQ sits at the bottom, with your neighbours to
the left and right.

Every clickable control lives inside the circle. In a match, the selected
unit's menu is a row of ring segments just inside the band (radius 350 to
400), on the lower arc by my HQ: with the HQ selected, production and cancel;
with units selected, attack, stop, and for utilities turret and deploy. Each
segment names its action along the arc, with its key and price.

Drag-selecting draws a rectangle in polar coordinates around the ring's
centre: two sides are radii and two are arcs, which fits a ring far better
than a screen rectangle.

In the lobby the ring shows who would sit where, and it animates: a new
player's sector opens and pushes the others aside, and a leaver's closes.
Players' names run along the ring beyond the map, each in their own sector,
flipping on the lower half of the screen so they never read upside down.

When the game opens, the circle and everything in it grows out of the centre
to full size over one second, easing out.

## Cake mode and window mode

On desktop the game runs in **cake mode**: it *is* the circle, a
frameless, transparent window of which only a disk shows. Its outer ring
(radius 512 to 600 of 600) simply continues the nebula, and is the window
chrome:

- drag the ring to move the window;
- drag its outermost band to resize, in the direction of the edge grabbed;
- sections of the ring on its top arc, marked off by thin lines, each with
  a large icon: close, maximise, minimise, pin always-on-top, and switch to
  window mode. A section lights up under the pointer, and players' names
  step aside for them.

A move or resize hands the pointer to the compositor, which keeps the
button's release to itself. So the game lets go of the button the moment it
asks for the move (and forgets all buttons whenever the pointer re-enters the
window): a button left "held" would make the next real press not count, and
every action on the frame would take two clicks.

What the platform allows, as found for Bevy 0.19 / winit 0.30:

| Capability | Support |
|---|---|
| Frameless, transparent window | Linux Wayland and X11 with a compositor, macOS. On Windows the corners stay black: its swapchains are opaque. |
| Drag-move, drag-resize, minimise, maximise from our own chrome | Everywhere on desktop (`start_drag_move`, `start_drag_resize`). |
| Always on top | winit can't on Wayland. On KDE Plasma the pin runs a one-line KWin script over D-Bus that sets `keepAbove` on this process's window; on other Wayland desktops the pin is dimmed. |
| Clicks passing through the transparent corners | No. Hit-testing is all-or-nothing for the whole window, so the corners of the square window still take clicks. |

Two Bevy details make the transparency work. The overlay camera copies to
the window with a replace blend, since later cameras on a window are
alpha-blended by default. The outside of the circle is cleared by a tiny
custom material, because `ColorMaterial` forces alpha to 1 in opaque mode.
`cargo run -p cake-bevy --example surface_probe` prints what a machine's
surface supports.

**Window mode**, an ordinary decorated window, is the alternative: switch
with the section on the ring or the button in the lobby. The choice is saved
in `~/.config/cake/settings`, and `CAKE_MODE=window` or `CAKE_MODE=cake`
overrides it for one run. The web build is always an ordinary page.

## Background

The circle's background is a faint procedural nebula, made with btl's algorithm
(github.com/barafael/btl): three random expression trees, one per colour
channel, grown from a weighted grammar, compiled to stack bytecode and
evaluated per pixel over `(x, y, t)`. `t` follows a one-minute sine, so the
field slowly wavers back and forth. The seed is the room name, so everyone
in a room sees the same sky.

It fills the whole circle and stays put while the map zooms: a backdrop
camera draws a dark disk and the nebula behind the map, and the overlay draws
the same texture again on the ring beyond the map, mapped so the two meet
without a seam.

## Architecture

### Networking: host-sequenced lockstep

Multiplayer works like chinese-checke.rs: `bevy_matchbox` WebRTC
peer-to-peer, a signaling server only for introductions, and the peer with
the smallest id as host. Chinese checkers orders moves; cake orders **ticks**.

1. A guest sends each command to the host only.
2. The host is the clock. Every tick (20 Hz) it cuts a **turn**: every
   command that arrived since the last one, tagged with the sender's seat
   *from the roster* (never from the message), and sends it to everyone. It
   cuts turns even when they are empty.
3. Every peer, the host included, advances only by applying turns in order.
   The host's own commands and its bots' go through the same sequencer.
4. Guests report a checksum every 20 ticks, and the host flags any mismatch
   as a desync.
5. A solo player is its own host, through the same code path.

The host never waits for guests; a lagging guest only delays its own view,
then catches up by running extra ticks. Guests buffer about two turns to
smooth out network jitter.

### Deterministic simulation

The simulation (`cake-core`) has no engine dependency and no floats:

- positions are polar: an angle as a `u32` (a full turn is 2³², so
  wrap-around is free) and a radius in integer milli-units;
- distance uses the local tangent frame, `d² = (r̄·Δθ)² + Δr²`, in `i64`,
  so movement follows the ring and "pathfinding" is the shortest signed Δθ;
- entities are kept in id order, every scan runs in that order, and ties
  break on id. No hash maps, no randomness;
- the checksum hashes the postcard encoding of the whole state, which is
  platform-independent.

### Crates

| Crate | Role |
|---|---|
| `cake-core` | Rules and simulation. Engine-free and deterministic. |
| `cake-net` | Wire protocol, turn buffer, sequencer, checksum log, room names. |
| `cake-ai` | Scripted bots that play through ordinary commands. |
| `cake-bevy` | Lobby, lockstep systems, rendering (gizmo polylines), input, HUD. |

## Risks and open questions

- **Turtling:** everyone waits for the others to fight. Contested economy
  sites are the main counter-pressure; watch whether it is enough.
- **Kingmaking** by players who are already losing.
- **Two players** make a ring that is really two fronts against the same
  opponent.
- **Host advantage:** the host's commands have zero latency. A fixed input
  delay for the host would even this out.
- **Map hacks:** every peer holds the whole state.
- **Snowballing** from contested economy sites.
- **Desync recovery:** M1 only detects divergence. Recovery would resend the
  state from the host.

## Backlog (M2 and beyond)

- In-game chat, including whispers sent directly peer-to-peer (so they really
  are private), and map pings.
- Control groups.
- Ghosts: the last-seen position of enemy structures.
- Host migration and reconnecting mid-match; late join as a watcher.
- Replays from the turn log (the seed is just the roster).
- GitHub Pages deployment of the web build.
- Sound.
- A spatial index for the simulation (a sort by angle is enough on a ring).
