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
4. **Everything is circular.** The window is a circle, buttons are segments of
   rings, text runs along arcs, and even the selection box is a rectangle in
   polar coordinates. Nothing on screen is a rectangle if it can be a piece
   of a ring instead.

## Geometry

The map is an annulus centred on the origin: inner radius **400**, outer
radius **500**. With N players, seat `i` starts in the sector centred at angle
`i·τ/N`, and its HQ sits there at radius 450.

**Sectors are only where players start.** Nothing in the rules knows about
them: anyone may build anywhere on the ring, except within 100 of an enemy
HQ. Land is held implicitly, by whoever builds on it and can defend it. The
sector markers stay on the ring as a picture of the starting layout.

| | Inner edge | Midline | Outer edge |
|---|---|---|---|
| Circumference | ~2513 | ~2827 | ~3142 |

At N = 6, HQs are about 471 units apart along the midline.

**Neighbours.** Reaching a non-neighbour means marching through someone's
territory. That gives "let me pass" and "hit him, not me" deals real stakes.
When a player is eliminated the ring closes, and their two neighbours now
border each other. Everything the fallen player had goes with them, so
their land is **free for anyone to claim**: their economy sites are open
again, and whoever moves in first with economy buildings or turrets holds
it.

**Band width vs. weapon range.** The band is 100 wide, and that makes turret
range a key number:

- gun turret range is 55, measured to the target's edge. A gun at mid-band
  (r = 450) just reaches both edges of the band, but only across a gate about
  46 units long;
- a raider hugging the edge is under fire for well under a second, so it
  can slip past a single turret;
- two or three turrets, or a screen of brawlers, seal a front. The plasma
  turret (70) and the missile turret (95) reach across the whole band, at a
  price.

This is deliberate: raids should be possible but answerable.

**Economy scarcity.** Economy buildings keep at least **150** from every other
economy building (anyone's) and from every HQ.

- At N = 6 that leaves about two safe sites per player (at their HQ ± 150)
  and about one **contested site at each border**.
- So growing your economy means building toward your neighbours, and border
  land is worth fighting over. A fallen player's land is the biggest prize
  of all.
- Staggering buildings radially packs a few more in. That is a knob to tune.

## Economy

- One resource, **supply**. No harvesting.
- Income: the HQ gives **5/s**, each economy building adds **2/s**.
- Pace: each economy building also makes the HQ build **15%** faster
  (shown as *Pace* in the readout). One production queue at the base pace
  can spend little more than the HQ's own income, so without this, land
  beyond two or three economy buildings would be worth nothing.
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

**Shots take time.** Only the brawler's blow lands at once. Everything else
fires a projectile that the simulation flies tick by tick, and the damage
lands when it arrives:

| Shot | Who | Flight |
|---|---|---|
| Blow | Brawler | Instant, melee |
| Bullet | Skirmisher (500/s), raider (420/s), gun turret (600/s) | Fast, and follows its target |
| Plasma | Plasma turret | A slow ball (60/s) aimed where the target *was*; it bursts there and hurts every enemy within 28, fully at the centre and half at the edge. Units that move can dodge it; clumps can't. |
| Missile | HQ (200/s), missile turret (220/s) | Launched off to one side, then steers toward its target, turning part of the way each tick; out of fuel after 4 s, it bursts harmlessly |

A bullet or missile whose target dies on the way bursts where it arrives,
harmlessly.

What those numbers mean in practice:

- a raider crosses half the ring (~1414) in about 22 s; a brawler takes
  about 50 s;
- at N = 6 a raider reaches a neighbour's HQ in about 7 s, a brawler in 17 s;
- the rules tests fight equal-cost armies (360 supply) at a neutral point, and
  each counter wins outright with no micro.

### Utility

- **Radar** (220): enemies inside it but outside anyone's vision show as blips.
- **Build a turret** of one of three kinds (see Structures). The utility walks
  to the site, lays a frame (10% HP), and builds it. A frame does not shoot.
  Other utilities can help by repairing it.
- **Repair:** 15 HP/s to a friendly entity within 30, free.
- **Deploy:** walk to a site and spend 5 s becoming an **economy building**.
  The utility is consumed. The site is checked when deploying starts and again
  when it ends.

### Structures

| Structure | Cost | Build | HP | Weapon | Vision | Notes |
|---|---|---|---|---|---|---|
| HQ | – | – | 2000 | missiles, range 60, 10 per 1 s | 120 | Fends off a raider or two, not an army |
| Gun turret | 75 | 8 s | 500 | bullets, range 55, 5 per 0.35 s | 70 | Rapid fire, the cheap front |
| Plasma turret | 100 | 10 s | 450 | plasma, range 70, 36 per 2.5 s, splash 28 | 90 | Breaks up clumps and brawler walls |
| Missile turret | 110 | 10 s | 400 | missiles, range 95, 24 per 1.8 s | 110 | Outranges skirmishers; sees far |
| Economy | – | 5 s | 400 | – | 50 | +2 supply/s |

No structure may be placed within 100 of an enemy HQ.

### Explosions

A building that is destroyed explodes, and throws every unit near it (anyone's)
away from it: harder the closer the unit stood, and the lighter it is.

| Blast | Radius | Force |
|---|---|---|
| HQ | 110 | 30 |
| Economy, plasma turret | 70 | 18 |
| Gun and missile turrets | 60 | 14 |

Units weigh: brawler 6, skirmisher and utility 3, raider 2. A thrown unit
starts at `force × (1 − d/R) / mass` per tick and slides, losing a quarter of
its speed each tick, so it travels about four times that. A raider beside a
falling HQ is flung about 60; a brawler, about 20. It stays in the band and
keeps its orders, so once it stops sliding it carries on.

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
wins. Target match length: 15–25 minutes (see Bots for how bot matches
measure up).

## Controls

Classic RTS micro:

| Input | Effect |
|---|---|
| Left click / drag | Select (Shift adds) |
| Right click | Move; attack an enemy; utilities repair a friendly; HQ alone: set rally |
| A, then click | Attack-move |
| S | Stop |
| B / N / M, then click | Utility builds a gun / plasma / missile turret |
| D, then click | Utility deploys into an economy building |
| Q W E R | Queue brawler, skirmisher, raider, utility |
| X | Cancel the last queued unit |
| Ctrl+A / Space | Select the army / the HQ |
| Wheel, middle-drag, arrows, H | Zoom, pan, home view |

The view is rotated so your HQ sits at the bottom, with your neighbours to
the left and right.

## Interface

Every clickable control lives inside the circle, and every button is a
**segment**: a slice of a ring that names what it does along its arc, with
its key (and price) beneath. A segment lights up under the pointer and fades
when it can't be used. They live on the segment ring (radius 335 to 385),
set clearly inside the inner circle with a gap to the map's band:

| Where | Lobby | Match |
|---|---|---|
| Top of the inner ring | Cake mode / window mode | Back to the lobby, once the match is over |
| Bottom of the inner ring, by my HQ | Add bot, remove bot, watch or play, start | The selected unit's menu: with the HQ, production and cancel; with units, attack and stop, and for utilities the three turrets and deploy. Once the match is over: the recap's metrics |

Text follows the circle too. In the lobby the title and tagline curve along
the top. Players' names run along the ring beyond the map, each in their own
sector, flipping on the lower half of the screen so they never read upside
down, and stepping aside for the window buttons. Only running prose (the
room, the economy readout, hints) stays in straight lines in the middle.

Drag-selecting draws a rectangle in polar coordinates around the ring's
centre: two sides are radii and two are arcs, which fits a ring far better
than a screen rectangle.

In the lobby the map's ring shows who would sit where, and it animates: a
new player's sector opens and pushes the others aside, and a leaver's
closes.

When the game opens, the circle and everything in it grows out of the centre
to full size over one second, easing out.

The circle is a round window onto the game, in either mode. At first the
map, the nebula behind it, the players' names and the readouts and menus in
the middle fill it exactly. Zooming and panning take all of them along
together, like looking closer at the cake; only the window's controls stay
where they are. Whatever would show past the circle's edge is masked, so
the view stays round however far it zooms.

### Effects

The map is still all lines, but fighting is lit up. Nothing here is
simulated: effects read the simulation's events and its projectiles and are
decoration on top, with their own randomness. What should shine is drawn
twice, thin and bright over wide and faint, which reads as a glow.

- **In flight:** bullets are short bright streaks; plasma balls swell as they
  fly, throb, and trail sparks, with wisps circling them; missiles are small
  darts with a flickering flame and a smoke trail that shows how they curved.
- **Firing:** gun flashes at the muzzle, a ring as plasma leaves the emitter,
  a puff of smoke as a missile launches, and a brawler's blow as a swipe
  across its front.
- **Landing:** sparks for bullets; for plasma, a ring widening to the splash
  radius and a spray of embers; for missiles, a small fireball with sparks and
  smoke.
- **Hits:** whatever is hit flashes white for a moment.
- **Deaths:** the outline breaks into its line segments, which tumble apart
  and fade. Buildings also burst into embers and smoke, and their blast sends
  a shockwave out to exactly the radius it throws units, with a ring of dust.
  Thrown units trail streaks while they slide.
- **Damage:** buildings under half health smoke, and under a quarter they
  spark. A plasma turret's core glows brighter as it charges.

Effects show only where I can see (my own losses always show), so they give
nothing away through the fog.

### Recap

When a match is decided, the middle of the ring fills with a chart of how it
went. Time runs clockwise around the circle from the top back round to it,
with a gap at the top for the scale; a value is a radius, from a baseline at
125 out to 285. Each player is a curve in their colour, and a cross marks
where a player was knocked out. The curves draw themselves in a sweep, the
verdict runs along the top, and the metrics are segments along the bottom
(keys 1 to 5):

- **Units:** mobile units on the ring;
- **Army:** what the fighting units cost;
- **Income:** supply per second;
- **Reach:** the share of the ring in sight, as the union of what each
  unit and building sees;
- **Losses:** supply's worth of units and buildings lost so far.

The middle lists everyone, best first, at the end or at the moment under the
pointer, and the chart marks that moment with a line and a dot on each curve.

The record (`cake-core::history`) samples every second and halves its
resolution whenever it passes 720 samples, so a long match stays small. It
watches the simulation from outside and is not part of the checksum.

## Cake mode and window mode

On desktop the game runs in **cake mode**: it *is* the circle, a
frameless, transparent window of which only a disk shows. Its outer ring
(radius 512 to 600 of 600) is the window chrome. At the fitted view it holds
the players' names over the rim of the nebula; zoomed in, the map shows
through it, but the pointer there is still the chrome's:

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

It belongs to the circle: at the fitted view it fills it, and it zooms and
pans with the map. A cake camera draws it, and the players' names, the
menus and the recap, behind the map; it follows the map camera's pan and
zoom but not its turn, so the sky and the words stay upright. The readouts
in the middle are Bevy UI, which is laid out in its own pixels, so they
follow by other means: the UI scale tracks the zoom and the box they sit in
is moved with the pan. Behind that, a fixed backdrop draws a dark disk that
fills the round viewport, which shows around the nebula when the view is
zoomed out.

Text is rasterised at its font size whatever the camera does, so the names
and the recap's text are rasterised at a power of two times their size that
follows the zoom, and scaled back down: sharp up close, and only a few
sizes in the font atlas.

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
| `cake-core` | Rules and simulation, and the match's history for the recap. Engine-free and deterministic. |
| `cake-net` | Wire protocol, turn buffer, sequencer, checksum log, room names. |
| `cake-ai` | Scripted bots that play through ordinary commands. |
| `cake-bevy` | Lobby, lockstep systems, rendering (gizmo polylines), effects, input, HUD, recap. |

## Bots

Bots are seats like any other: they read the simulation and send ordinary
commands, which the host sequences. They exist to fill the ring and to make
a bots-only match worth watching, so each one is a character. The host names
bots after their temperament when the match starts ("Turtle", "Raider 2").

| Temperament | Plays |
|---|---|
| Warlord | Early, brawler-heavy waves; small garrison; fights to the last; vengeful; a gun turret |
| Raider | Raider-heavy; packs of three go for economy and flee when hurt; a missile turret |
| Turtle | Four turrets (plasma, missile, gun, in turn), a big garrison, late but large waves; vengeful |
| Balanced | An even mix, medium waves; gun and plasma turrets |

All of them:

- **Hold land implicitly.** A bot's land is the stretch of ring nearer its HQ
  than any other living HQ. When a neighbour falls, the land between the two
  survivors is split afresh, and a bot claims its new ground: it wants more
  economy buildings the more land it holds (up to three times its
  temperament's) and builds them out into the gap.
- **Fight with zeal when invaded.** Any enemy at home (on its land, as far
  out as a starting sector reaches) or near one of its buildings brings the
  whole home army down on it, and a serious invasion calls a running attack
  back home. Production turns to counters of what came in. Defenders chase
  invaders out, then return. Claimed ground further out is defended where
  it has buildings; otherwise two survivors sharing the ring would each call
  every attack home the moment it crossed the middle.
- **Watch their neighbours.** A running "pressure" measures how much of each
  neighbour's army has been at home lately; heavy pressure buys
  an extra turret on that front. Production tilts toward counters of what
  the neighbours field, weighted by how hard each presses. The vengeful keep
  a grudge against the last invader and counterattack sooner.
- **Attack in waves**, leaving a garrison, going for the target's nearest
  building first, so fights start at the borders and push in. Each wave
  waits for a bigger army, but a bot that has had a first wave's worth at
  home for a minute without getting there goes with what it has (losses can
  keep an army from ever growing). A beaten wave turns back.

For calibration: in a six-bot match first blood comes after about a
minute, and matches end after six to fifteen minutes; winners end up
holding eight or nine economy buildings, most of them on claimed land.

## Risks and open questions

- **Turtling:** everyone waits for the others to fight. Contested economy
  sites are the main counter-pressure; watch whether it is enough.
- **Kingmaking** by players who are already losing.
- **Two players** make a ring that is really two fronts against the same
  opponent.
- **Host advantage:** the host's commands have zero latency. A fixed input
  delay for the host would even this out.
- **Map hacks:** every peer holds the whole state.
- **Snowballing** from contested economy sites and a fallen player's land:
  economy buys both income and production pace, so a lead compounds. The
  15% pace per building is the knob.
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
- A spatial index for targeting too: vision and separation already scan only a window over the angle order.
