# Local development

You can run the whole app without an AWS account: moto emulates DynamoDB and S3, a local binary serves the same axum router on :9000, and the PWA runs in "dev auth" mode (you type a user id instead of using Cognito).

## Toolchain: build in WSL / Linux / macOS

Lambda runs on Linux, and the AWS SDK's crypto (`aws-lc-sys`) needs a C toolchain. On this Windows machine the `x86_64-pc-windows-gnu` Rust toolchain has no MinGW binutils, so even `cargo check` fails with `error calling dlltool 'dlltool.exe'`. Options:

1. **Recommended: WSL Ubuntu** (already installed, with Rust and gcc). Keep a copy of the repo in the Linux filesystem for speed (`~/ptapp`). `D:` isn't currently mounted in WSL; mount it with `sudo mkdir -p /mnt/d && sudo mount -t drvfs D: /mnt/d`, or run `wsl --shutdown` and reopen WSL.
2. Native Windows: install *Visual Studio Build Tools (C++ workload)* and run `rustup default stable-x86_64-pc-windows-msvc`. That's enough for `cargo test`; cross-building the Lambda still needs `cargo-lambda` (which bundles Zig).
3. CI (GitHub Actions `ubuntu-latest`) for builds and deploys.

Install the tools (WSL / Linux):

```bash
rustup target add aarch64-unknown-linux-gnu
pip install --user cargo-lambda aws-sam-cli cfn-lint   # cargo-lambda brings Zig for cross-compiling to arm64
```

The AWS emulator (moto) runs in Docker, so you also need Docker Desktop (Windows/macOS, `winget install Docker.DockerDesktop`) or Docker Engine (Linux).

## Run everything

```bash
# 1. AWS emulator (moto in Docker, on :5055; see docker-compose.yml)
docker compose up -d --wait

# 2. API on http://localhost:9000 (creates table + bucket in moto, DEV_AUTH on)
AWS_ENDPOINT_URL=http://127.0.0.1:5055 AWS_REGION=us-east-1 \
AWS_ACCESS_KEY_ID=test AWS_SECRET_ACCESS_KEY=test RUST_LOG=info \
cargo run -p api --bin local &

# 3. Web app on http://localhost:8080
python3 -m http.server 8080 -d web
```

Open <http://localhost:8080>, click **Sign in**, and enter a user id such as `coach1`. Register as a coach and create an invite. In a private window, open the invite link (`#/join/CODE`) and sign in as `client1`.

`web/js/config.js` is the dev config (`devUser: true`, `apiUrl: http://localhost:9000`). The deploy script overwrites it. **Never deploy with `devUser: true`**; the deployed Lambda ignores the dev header anyway because `DEV_AUTH` is not set there.

Photo and video uploads in dev go to moto's S3. The browser PUT to moto may hit CORS limits; test uploads against a real dev stack.

## Tests

```bash
cargo test                                   # unit tests (core + api)
E2E_ENDPOINT=http://127.0.0.1:5055 cargo test -p api --test e2e
cargo clippy --all-targets
cfn-lint template.yaml
```

The e2e test (`crates/api/tests/e2e.rs`) drives the real router in process through onboarding, invites, intake/waiver, programs, assignment, offline-conflict logging, dashboard maths, skinfolds, photos, comments, tenant isolation and the audit log.

Browser journey (Playwright; uses installed Microsoft Edge by default, `BROWSER_CHANNEL=chrome` for Chrome):

```bash
python -m venv .venv && .venv/Scripts/pip install playwright   # Windows paths shown
.venv/Scripts/python tests/ui/journey.py                         # needs steps 1-3 above running
```

It signs up a coach, invites a client, builds a 2-week program (straight sets + EMOM, week copy), and has the client join, complete intake and waiver, and log a workout **while offline**. It then checks that the log syncs on reconnect, logs weights and a skinfold, and verifies the coach dashboard, set-level comment, roster pain flag and client message thread. Screenshots go to `tests/ui/shots/`.

## Code map

| Change | Where |
|---|---|
| A domain rule or calculation | `crates/core/src/*` plus a unit test |
| A new endpoint | handler in `crates/api/src/routes/<area>.rs`, route in `crates/api/src/lib.rs`, a step in `tests/e2e.rs` |
| A new key pattern | `crates/api/src/store.rs` header comment and `docs/ARCHITECTURE.md` |
| A new screen | `web/js/views/<area>.js`, route in `web/js/app.js`, file added to `SHELL` in `web/sw.js` |
