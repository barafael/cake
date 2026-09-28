# cake

A real-time strategy game on a ring: up to eight players, two neighbours
each, and promises nobody has to keep. See [DESIGN.md](DESIGN.md) for the
game and the architecture.

## Run

```sh
cargo run -p cake-bevy                       # opens the lobby in a fresh room
CAKE_ROOM=myroom cargo run -p cake-bevy      # join a named room
CAKE_BOTS=5 cargo run -p cake-bevy           # skip the lobby: you and 5 bots
CAKE_BOTS=6 CAKE_WATCH=1 cargo run -p cake-bevy  # watch 6 bots play
```

On desktop the game runs in **cake mode**: the window is a frameless circle.
Drag its outer ring to move it, drag the ring's outermost edge to resize,
and use the sections on top to close, maximise, minimise, pin, or switch to
**window mode**, an ordinary decorated window. The choice is saved in
`~/.config/cake/settings`; `CAKE_MODE=window` or `CAKE_MODE=cake` overrides
it for a run.

Everyone who opens the same room meets in its lobby. The peer with the
smallest id hosts: it adds bots (`B`/`X`) and starts the match (`Enter`).
`W` switches between playing and watching. `CAKE_NAME` sets your name.

Peers are introduced by a matchbox signaling server
(`wss://omdurman-matchbox.fly.dev` unless `MATCHBOX_SERVER` is set at build
time). For local testing run `matchbox_server` and build with
`MATCHBOX_SERVER=ws://127.0.0.1:3536`.

The web build is `trunk serve` (or `trunk build`); the room travels in the
URL as `#room=name`.

`CAKE_SHOT=out.png` (with `CAKE_SHOT_AT=seconds`) saves one screenshot of
the window, alpha included, which is handy for checking the circle's
transparency.

### Recording a demo

A watched bot match can be recorded to video, hands off (needs `ffmpeg`):

```sh
CAKE_BOTS=3 CAKE_WATCH=1 CAKE_TEMPERS=turtle,warlord,raider CAKE_SPEED=2.5 \
  CAKE_RECORD=fortress.mp4 CAKE_RECORD_TITLE="Fortress" cargo run -p cake-bevy
```

- `CAKE_RECORD` captures every frame from the start of the match, at a fixed
  30 frames per second of game time, so the video keeps its pace however slow
  capturing is. The mouse and keyboard are ignored meanwhile. A director
  follows the heaviest fighting, and once the match is decided the recap
  shows each metric in turn before the app quits.
- `CAKE_RECORD_TITLE` is shown in the middle during wide shots, and
  `CAKE_RECORD_SECS` caps the length.
- `CAKE_SPEED=n` runs the match n times as fast; `CAKE_TEMPERS` picks the
  bots' temperaments seat by seat (`warlord`, `raider`, `turtle`,
  `balanced`).

## Test

```sh
cargo test --workspace
# Three real peers over WebRTC, with an in-process signaling server:
cargo test -p cake-bevy --test multiplayer -- --ignored --nocapture
```
