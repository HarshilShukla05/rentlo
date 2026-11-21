# Rentlo handoff

Read this file first, then `docs/PLAN.md` (the decisions). Together they
are enough to continue with no chat history. Keep this file current: update
it in the same commit as the work it describes.

Last update: 2026-11-21. Branch: `claude/gallant-darwin-gxloil`.

## 1. Product in one paragraph

WhatsApp-first flatmate matching for India. A person sends "Hi" to the bot
number, describes themselves in plain words, and the bot builds a profile,
shows match cards ("1 yes" / "1 skip"), and on a mutual yes gives each side
the other's WhatsApp link. A public live map (PWA) shows blurred pins.
No app install, no login screen. Details and every decision: `docs/PLAN.md`.

## 2. Status

| Phase (PLAN §15) | State | Proof |
|---|---|---|
| 1 `domain`: types, matching, blur, rule extractor, chat state machine | Done | 62 unit tests |
| 2 `app` use cases + `store` (PostgreSQL) | Done | 9 unit + 11 end-to-end tests on real PostgreSQL |
| 3 `wa` WhatsApp adapter + `rentlo bot` binary + Dockerfile + `inframan.yaml` + CI | Done, **not deployed** | Compiles, lint-clean, starts and migrates locally |
| 4 `rentlo api` (HTTP + SSE) | Not started | Use cases exist in `crates/app/src/web.rs` |
| 5 `web/` map PWA | Not started | |
| 6 `llm` extractor via Inframan agent gateway | Not started | Bot uses `RulesExtractor` (no AI) today |
| 7 Deploy to Inframan | Application created; first release running | See section 6 |

**Milestone 1** (first thing the owner tests): send "Hi" to the bot number,
two people get matched and exchange contacts. Needs only phase 7.
**Milestone 2**: the live map at `https://rentlo.inframan.live`. Needs 4, 5.

## 3. Code map

```
crates/domain   Pure rules. No I/O, no clock, no randomness.
  profile.rs      ProfilePatch (draft, all optional) -> ProfileData (complete). Budget, enums.
  matching.rs     eligible() gate, score() 0-100, explain() viewer-facing reasons.
  extract.rs      Rule-based free text -> ProfilePatch. Also phone().
  command.rs      Fixed commands (hi, help, 1 yes, skip CODE, I like CODE, delete yes, ...).
  conversation.rs Chat state: stage, draft, what was asked, cards shown.
  pairing.rs      Match + decide(): first answer final, skip closes the pair.
  geo.rs city.rs ids.rs
crates/app      Use cases against ports. Knows no DB/WhatsApp/LLM.
  ports.rs        Store/Tx (one transaction per use case), Messenger, ProfileExtractor.
  bot.rs          One inbound message -> state change + replies, one transaction.
  ops.rs          publish (save + pair + alerts), decide, tokens. Shared by bot and web.
  dispatch.rs     Outbox sender, retries with backoff.
  web.rs          Map pins (blurred), cards, magic-link sign-in, me, matches, edit.
  copy.rs         Every user-facing sentence. policy.rs: caps, quiet hours, TTLs.
crates/store    PostgreSQL adapter (sqlx 0.9, runtime-checked SQL).
  migrations/0001_init.sql  Schema, outbox + pins NOTIFY triggers.
  tests/flow.rs   End-to-end scenarios. Read these to learn the behaviour.
crates/wa       whatsapp-rust 0.7: inbound DMs -> app::Inbound; Link = Messenger.
crates/rentlo   Binary. `rentlo bot`, `rentlo migrate`. Command from argv or RENTLO_COMMAND.
```

Dependency rule: `domain` <- `app` <- (`store`, `wa`, `llm`) <- `rentlo`.

## 4. How to run and test

```bash
# Local PostgreSQL for tests (any server where the user may CREATE DATABASE)
export RENTLO_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:54329/postgres
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # without the variable, the 11 DB tests print SKIPPED
```

In the Claude cloud sandbox the server is started like this (data must not
live in the scratchpad: a worker restart makes it unreadable):

```bash
D=/var/lib/postgresql/rentlo-test
su postgres -c "/usr/lib/postgresql/16/bin/pg_ctl -D $D -o '-p 54329 -c listen_addresses=127.0.0.1 -c unix_socket_directories=/tmp -c fsync=off' -l $D/log.txt start"
```

Run the bot locally: `DATABASE_URL=... WA_SESSION=./wa.db cargo run -p rentlo -- bot`.
The cloud sandbox cannot reach WhatsApp directly, so no link code appears
there. In the cluster it connects directly.

Env vars: `DATABASE_URL` (required), `PUBLIC_URL`, `BOT_PHONE`, `WA_SESSION`
(default `/data/whatsapp.db`), `WA_PAIRING_NUMBER` (digits; link by code
instead of QR), `RUST_LOG`, `RENTLO_COMMAND`.

## 5. Invariants. Do not break these.

1. One use case = one transaction. State and the outbox rows it causes commit together.
2. Inbound message ids are recorded in that transaction: redelivery is a no-op.
3. Every outbox row has a `(topic, dedupe_key)`; enqueue is idempotent.
4. Exactly one `bot` replica (`scaling: off`, `min_replicas: 1`). Two would both answer.
5. No endpoint or message reveals a phone number before a mutual yes, or an exact pin ever. Pins go through `geo::blur`.
6. A first decision on a pair is final. A skip is silent to the other side.
7. Unprompted messages: max 3 alerts/day, held to 08:00-22:00 India time (`policy.rs`).
8. `domain` stays pure. Time and randomness are arguments.
9. Every sentence a user reads lives in `copy.rs`.

## 6. Next steps, in order

### Owner (human) steps. Nothing deploys until 1 is done.

1. Done 2026-11-21: `inframan-git` reaches `HarshilShukla05/rentlo`.
2. Pending: the owner will get a secondary WhatsApp number. Until it is
   linked, the bot runs but stays unlinked and answers nobody.
   Decide which WhatsApp number the bot uses. It answers **every direct
   message** to that number. Do not use a personal number. If it is the
   Smash Karts bot number, both bots share one ban risk.
3. Set `WA_PAIRING_NUMBER` (that number, digits) on the `bot` service in the
   Inframan console. It is personal data: never commit it.
4. After the first deploy, read the `bot` logs for `enter XXXXXXXX` and type
   the code on the bot phone: WhatsApp → Linked devices → Link a device →
   Link with phone number instead. Codes expire in about 2 minutes; a new one follows.

### Agent steps

1. **Deploy (phase 7)**: done on Inframan. Read `inframan://instructions`
   before any Inframan work.
   - Infrastructure `Prod` `f498af8b-46c1-4a34-9c5e-c75546d1f819` (asia-south1).
   - Pipeline `d9d4825c-38d5-43a7-beaa-ec26c0ad0fd1`: push to
     `claude/gallant-darwin-gxloil` releases. Application `rentlo`
     `0073d5b4-a8de-49a6-babf-ea48a0ea8271`, GitHub app id 2.
   - A push releases only if it changes `inframan.yaml` data or a watched
     path (`crates`, `Cargo.*`, `rust-toolchain.toml`). Docs-only pushes do not.
   - When a `main` branch exists: create a pipeline for `main` and
     `update_application` the branch.
   - Logs: `get_logs` application `rentlo`, service `bot`.
2. **`rentlo api` (phase 4)**: axum, port 8080, routes in PLAN §10, all backed
   by `app::Web`. SSE `/api/v1/map/stream`: `PgStore::listen(&[PINS_CHANNEL])`,
   payload is a profile code, call `Web::pin(code)`: `Some` -> `pin.upsert`,
   `None` -> `pin.remove`. Cookie `rentlo_session` (HttpOnly, Secure,
   SameSite=Lax, 30 days). `/m/{token}` page posts the token to `POST /api/v1/session`.
   Serve `web/dist` as static files. Add an `api` service to `inframan.yaml`:
   `type: web`, `RENTLO_COMMAND: api`, `domain: rentlo.inframan.live`,
   `network.timeout_seconds: 600` (SSE), hpa 1-3.
   The domain must be created and **active** before the release names it
   (`add_domain`; DNS A record to `34.93.175.240`).
3. **`web/` PWA (phase 5)**: Vite + React + TS + MapLibre + OpenFreeMap. One
   screen: map, three filters, bottom sheet card, "Connect on WhatsApp" opens
   `https://wa.me/<BOT_PHONE>?text=Hi%20Rentlo%2C%20I%20like%20<CODE>`
   (the bot parses that as `Command::Interest`). Docker: add a node build stage.
4. **`llm` crate (phase 6)**: implement `ProfileExtractor`. Call the Inframan
   agent gateway in Anthropic Messages format with one forced tool whose
   schema is `ProfilePatch` (enums from `Type::names()`). 8 s timeout; on any
   failure return `domain::extract::extract(...)`. Merge rule: the rules'
   result is the base, the model's fields override only where present.
   Check how a non-agent service reaches the gateway before building
   (`get_agent_connections`; agent services get `INFRAMAN_AGENT_MODEL_URL`).

## 7. Known gaps and risks

- **Phone may be unknown.** WhatsApp sometimes gives only a LID address. Then
  `users.phone` is empty and the intro says "they will message you". If both
  sides lack a phone, nobody can start the chat. The bot accepts a typed
  number (`extract::phone`) but does not ask for one yet. Fix: at a mutual
  yes, ask the side without a phone to send it, then send the intro.
- **Edit re-pairing.** An edit deletes undecided pairs and recomputes them.
  People already alerted are not alerted again, but their open card for the
  old pair is gone (they see the new pair under "matches").
- **Location pin is a live update only through chat.** The web edit endpoint
  accepts `pin` in the patch, but the PWA has no pin picker yet.
- **Unofficial WhatsApp client.** Ban risk. Move to the Cloud API (new
  `Messenger` adapter) before real scale. See PLAN §14.
- **Docker image never built here** (no Docker daemon in the sandbox). The
  release binary builds and links only libc/libm/libgcc. First real build is
  on Inframan.
- Number masking is planned as a separate central service (owner's
  decision, 2026-11-21): virtual numbers with two-way relay, shared by many
  apps. Rentlo will use it through a new `Messenger` adapter.
- `wa` has no unit tests. Its logic is thin; it is tested by running it.

## 8. Commit log so far

```
c0d0312 Add the WhatsApp adapter, the server binary and deploy files
235209b Drop an unused field from the flow tests
7bc40d2 Add use cases, the PostgreSQL store and end-to-end tests
67db334 Add the build plan and the domain crate
```
