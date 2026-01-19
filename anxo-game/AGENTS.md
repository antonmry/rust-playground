# AGENTS.md

## E2E validation (must run after changes)
- Always run the game after any change and confirm the level behavior is correct.
- Use the headless E2E flow with `xvfb-run` and capture a screenshot.

### Build (Linux target in this repo)
```bash
CARGO_TARGET_DIR=target-linux cargo build
```

### Run level 2 E2E (example: key + padlock)
```bash
ANXO_START_LEVEL=level2 \
ANXO_AUTORUN=1 \
ANXO_START_CODE="$(printf 'from game import hero, key, padlock\nhero.move_left()\nhero.pick(key)\nhero.move_right()\nhero.move_right()\nhero.move_right()\nhero.move_right()\nhero.open(padlock)\nhero.move_right()\n')" \
xvfb-run -s '-screen 0 1024x768x24' sh -c 'target-linux/debug/anxo-game & pid=$!; sleep 8; import -window root tmp/anxo-e2e.png; kill $pid'
```

### Notes
- If the level differs, update the `ANXO_START_LEVEL` and `ANXO_START_CODE` accordingly.
- Always include a fresh screenshot (`tmp/anxo-e2e.png`) and verify the expected success/error message appears. 
