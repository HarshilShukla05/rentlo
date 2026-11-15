# Rentlo: build plan

Status: decided. Every section below is a decision, not an option list.
An agent can build one section without reading the chat that produced it.

## 1. Core idea

A bad flatmate costs a year. People choose one after a single call.
Rentlo matches people on how they live, before they meet.

One sentence: **send "Hi" on WhatsApp, describe yourself in your own words, and
get flatmates who live like you.**

- No app install. No login screen. No swipe deck.
- WhatsApp is the main surface. The bot builds the profile from free text and
  pings the user when a match appears.
- A public web map (PWA) shows who is looking, live, for people who only want
  a quick look.

## 2. Decisions from the Q&A

| Topic | Decision |
|---|---|
| Main surface | WhatsApp chat with a bot |
| Second surface | Public real-time map, open to everyone, no account |
| User types | Has a room, needs a room, team up and find a flat later |
| Launch area | All major metros. Listings accepted from any city in India |
| Identity | WhatsApp number is the account. Web is anonymous until the bot sends a private link |
| Contact | Both people accept, then each gets a direct WhatsApp link to the other |
| WhatsApp transport | Unofficial WhatsApp Web client on the owner's number, as in the `-game` repo |
| Backend | Rust |
| Frontend | TypeScript PWA |
| AI access | Inframan agent gateway (Anthropic connection, already live) |
| Hosting | Inframan cluster `Prod`, region `asia-south1` |
| Address | `rentlo.inframan.live` |
| Revenue | None in v1. Later: verified listings, paid matching help |

### Identity, in detail

The owner asked for no sign-in barrier, and for the browser to supply the
user's email silently. A browser never gives a site the user's email without
a consent prompt. So the design uses what already exists:

1. A WhatsApp message proves control of a phone number. That number is the
   account. No OTP, no SMS cost, no password.
2. The web map needs no account to view.
3. To act on the web (edit profile, accept a match), the bot sends a private
   link. The link sets a session cookie. The user sees no login screen.
4. A map visitor with no account taps "Connect on WhatsApp". That opens
   WhatsApp with a prefilled message that carries the profile code. The first
   message creates the account.

## 3. Product scope

### v1 (this build)

1. WhatsApp onboarding by free text, with numbered-option fallback.
2. Profile: intent, city, area, budget, move-in date, gender, gender
   preference, sleep, cleanliness, food, smoking, drinking, pets, guests,
   work mode, short note.
3. Matching with a score, reasons, and probable clashes.
4. Match cards in WhatsApp. Accept or skip. Mutual accept reveals contact.
5. Match alerts when a new compatible profile appears, with a daily cap.
6. Public live map with area-level pins and filters.
7. Private link from the bot to the user's own page on the web.
8. Commands: `pause`, `resume`, `edit`, `delete`, `report`, `help`.

### Not in v1

Payments, ID checks, photos, in-app chat, call masking, landlord listings,
native apps, admin console. Each has a clean place to attach later (section 12).

## 4. User flows

### 4.1 WhatsApp onboarding

```
User: Hi
Bot:  Hi! I find flatmates who live like you. Tell me in one message:
      do you have a room or need one, which city and area, your budget,
      and how you live (sleep time, cleanliness, food, smoking, pets).
      By continuing you agree to rentlo.inframan.live/privacy
User: need a room in koramangala blr, 18k max, i sleep by 11, veg, no smoking, clean freak
Bot:  Got it:
      • Need a room · Bengaluru, Koramangala · up to ₹18,000
      • Sleeps early · very tidy · veg · non-smoker
      Two quick things: 1) your gender  2) flatmate gender preference (any / same)
User: male, any
Bot:  You are live. 3 matches now. (cards follow)
      Send 📍 a location pin of your preferred area to appear on the live map.
```

Rules:
- Required to go live: intent, city, budget, gender, gender preference.
- The bot asks only for what is missing. City and budget go in one message.
  A field with numbered options (intent, gender, gender preference) is asked
  alone, so "2" is never ambiguous.
- Every question accepts a short fixed reply, so the flow works with no AI.
- Gender and gender preference are asked at most twice. After that they
  default to "other" and "any". Intent, city and budget have no safe default,
  so the bot keeps asking for them.
- When the required fields are known and fewer than three habits are stated,
  the bot asks once how the person lives. Any reply moves on.

### 4.2 Match card and contact

```
Bot:  Match 1 of 3 · 87% · code R7K2Q
      Has a room · Koramangala · ₹16,000 · female · moves in 1 Dec
      Fits: both sleep early, both very tidy, both veg
      Watch: has a cat
      Reply  1 yes  or  1 skip
```

- A card never shows a name or a number.
- When both people say yes, each gets the other's first name and a `wa.me` link.
- A skip is final for that pair and silent to the other person.

### 4.3 Web map

- Opens straight on the map. No splash, no login.
- Pins are area-level (section 8). A pin opens a bottom sheet with the card.
- One button: "Connect on WhatsApp". It opens WhatsApp with
  `Hi Rentlo, I like R7K2Q`.
- Filters: intent, budget, gender preference. Three controls, no more.
- New pins appear live through a server-sent event stream.
- A signed-in user (through the private link) sees their own matches and can
  edit the profile.

## 5. Architecture

```
                 WhatsApp                      Browser (PWA)
                    │                               │
          ┌─────────▼─────────┐           ┌─────────▼─────────┐
          │  bot  (1 replica) │           │  api  (1..N)      │
          │  WhatsApp socket  │           │  HTTP + SSE       │
          │  conversation     │           │  static PWA files │
          │  outbox dispatcher│           │                   │
          └───┬───────────┬───┘           └─────────┬─────────┘
              │           │  LISTEN/NOTIFY          │
              │       ┌───▼─────────────────────────▼───┐
              │       │           PostgreSQL            │
              │       │  state + outbox (one database)  │
              │       └─────────────────────────────────┘
          ┌───▼────────────┐
          │ Inframan agent │  profile extraction only
          │ gateway (LLM)  │
          └────────────────┘
```

One Rust binary, three subcommands: `rentlo bot`, `rentlo api`, `rentlo migrate`.
Two services run the same image.

### 5.1 Why this shape

- **One source of truth.** PostgreSQL holds all state. Nothing is written to
  two systems in one step.
- **Transactional outbox.** A state change and the events it causes commit in
  one transaction. A dispatcher reads the outbox with
  `FOR UPDATE SKIP LOCKED`. `NOTIFY` wakes it at once. A slow poll is the
  safety net.
- **At-least-once, idempotent.** Each event has a dedupe key. Each handler is
  safe to run twice. Inbound WhatsApp messages dedupe on the message id.
- **No broker.** Kafka or Pub/Sub adds cost and operations for no gain at this
  size. The outbox table is the queue. If volume needs a broker later, the
  dispatcher is the only part that changes.
- **Cost follows traffic.** Two small Rust processes and one 256 MB
  PostgreSQL. The LLM is called only for free-text messages, never for
  matching or for numbered replies.
- **The bot is one replica.** One WhatsApp session can have one live socket.
  The API holds no session state and can scale.

### 5.2 Layers (ports and adapters)

```
crates/
  domain/   Pure logic. No I/O, no async, no clock. Types, validation,
            matching score, conversation state machine, location blur.
  app/      Use cases and ports (traits). Depends on domain only.
  store/    PostgreSQL adapter: repositories, outbox, LISTEN/NOTIFY, migrations.
  llm/      Profile extractor: Anthropic-format client + rule-based fallback.
  wa/       WhatsApp adapter (whatsapp-rust).
  rentlo/   Binary. HTTP routes, wiring, config, subcommands.
web/        Vite + React + TypeScript PWA.
```

Dependency rule: `domain` ← `app` ← adapters ← `rentlo`. Nothing points the
other way. `domain` and `app` never name PostgreSQL, WhatsApp or an LLM.

Ports in `app`:

| Port | Purpose | v1 adapter |
|---|---|---|
| `Store` | Users, profiles, matches, conversations, sessions | PostgreSQL |
| `Outbox` | Enqueue and claim events | PostgreSQL |
| `Messenger` | Send a WhatsApp text to a user | whatsapp-rust |
| `ProfileExtractor` | Free text to a profile patch | LLM, then rules |
| `Clock` | Current time | System clock |

The official WhatsApp Cloud API becomes a second `Messenger` adapter. Nothing
above the port changes.

## 6. Data model

PostgreSQL 18. No extensions. Money is whole rupees (`integer`). Time is
`timestamptz`. Ids are UUID v7.

```
users            id, wa_jid UNIQUE, phone, first_name, consent_at,
                 created_at, deleted_at
profiles         id, user_id UNIQUE, public_code UNIQUE, status,
                 intent, city, area, lat, lng, cell_lat, cell_lng,
                 budget_min, budget_max, move_in, gender, gender_pref,
                 lifestyle JSONB, note, version, created_at, updated_at
conversations    user_id PK, state JSONB, updated_at
inbound_messages wa_message_id PK, user_id, received_at
matches          id, profile_a, profile_b, score,
                 decision_a, decision_b, status, created_at, updated_at
                 UNIQUE (profile_a, profile_b), CHECK (profile_a < profile_b)
outbox           id BIGSERIAL, topic, dedupe_key, payload JSONB, attempts,
                 available_at, processed_at, last_error
                 UNIQUE (topic, dedupe_key)
magic_links      token_hash PK, user_id, expires_at, used_at
web_sessions     token_hash PK, user_id, expires_at
reports          id, reporter_user_id, profile_id, reason, created_at
```

- Columns that queries filter on are real columns. Lifestyle traits live in
  `lifestyle JSONB` because the trait list will grow. Unknown traits are
  ignored on read.
- `profiles.version` is an optimistic lock. A stale write fails and retries.
- A pair is stored once, with the smaller id as `profile_a`. Only the score
  is stored. The "fits" and "watch" lines are derived from the two profiles
  each time a card is drawn, because they are written for the viewer.
- Tokens are stored as SHA-256 hashes, never in clear.
- `delete` removes the profile, matches and conversation, and blanks the user
  row. The phone number is not kept.

## 7. Matching

Two stages. Both are deterministic. The LLM takes no part.

**Stage 1, hard filters (SQL).** Active profiles, another user, same city,
compatible intent, budgets overlap (10% tolerance), gender preference holds in
both directions, no earlier decision for the pair.

Compatible intents:

| | has room | needs room | team up |
|---|---|---|---|
| has room | no | yes | no |
| needs room | yes | no | yes |
| team up | no | yes | yes |

**Stage 2, score (pure Rust, `domain::matching`).** 0 to 100. Weights:

| Factor | Weight |
|---|---|
| Sleep schedule | 20 |
| Cleanliness | 20 |
| Smoking | 15 |
| Food | 10 |
| Budget closeness | 10 |
| Distance between area pins | 10 |
| Pets | 5 |
| Guests | 5 |
| Drinking | 3 |
| Work mode | 2 |

- A trait that one side did not state scores half, so thin profiles still match.
- The function also returns up to three "fits" and up to two "watch" lines.
  The text comes from fixed templates, so it is testable and costs nothing.
- Matches below 50 are not shown. Two profiles that state no habits score 55.
- A new or changed profile triggers matching for that profile only.

## 8. Location privacy

A room owner's pin is their home. The public map must never reveal it.

- The exact point is stored and used only for distance in matching.
- The API returns only a blurred point: the centre of a grid cell of about
  1.1 km (0.01°), plus a fixed offset derived from the profile id, kept inside
  the cell. The same profile always shows at the same blurred place.
- A profile with no pin is not drawn. It counts in the city total.
- No endpoint returns `lat` or `lng`.

## 9. AI use

One job: turn a free-text WhatsApp message into a profile patch.

- Call format: Anthropic Messages API with one forced tool whose input schema
  is the profile patch. Enums only. The domain layer validates every value again.
- Endpoint and model come from the environment. On Inframan the gateway
  supplies them and holds the credential. The service holds no key.
- Model: the small, fast tier. Extraction is a simple task.
- User text is data. The model has no tools except the patch schema, so text
  that tries to instruct it can change nothing but that user's own profile.
- If the call fails or times out (8 s), the rule-based extractor answers and
  the bot continues with numbered questions. The product works with no AI.
- A daily budget on the gateway caps spend.

## 10. API

JSON over HTTPS. Version prefix `/api/v1`.

| Method and path | Auth | Purpose |
|---|---|---|
| `GET /map/pins?bbox=&intent=&budget_max=&gender=` | none | Blurred pins in a box |
| `GET /map/stream` | none | SSE: `pin.upsert`, `pin.remove` |
| `GET /cities` | none | Active counts per city |
| `GET /profiles/{code}` | none | Public card |
| `GET /config` | none | Bot's WhatsApp link |
| `POST /session` `{token}` | none | Exchange private link for a cookie |
| `GET /me` | cookie | Own profile |
| `PATCH /me` | cookie | Edit own profile |
| `GET /me/matches` | cookie | Own matches |
| `POST /me/matches/{id}/decision` | cookie | `yes` or `skip` |
| `GET /healthz`, `GET /readyz` | none | Probes |

- Errors: `{"error": {"code": "...", "message": "..."}}` with a fixed code list.
- Cookie: `HttpOnly`, `Secure`, `SameSite=Lax`, 30 days.
- Public endpoints are rate-limited per IP. Pin queries are capped at 500 rows.
- SSE clients reconnect and reload the box. The stream carries no history.

## 11. Frontend

- Vite, React, TypeScript, MapLibre GL, OpenFreeMap tiles (no key, no cost).
- One screen: full-bleed map, a filter bar, a bottom sheet. No router pages
  except `/m/{token}` (private link) and `/privacy`.
- Installable PWA: manifest, service worker that caches the shell only.
- No component library. A few hand-written components and CSS variables.
- Look: dark city map, one accent colour, large tap targets, system font.
- The API serves the built files. One origin, no CORS.

## 12. Later work, and where it attaches

| Feature | Attach point |
|---|---|
| Official WhatsApp Cloud API | New `Messenger` adapter |
| Number masking, in-app relay | `Messenger` plus a `relay` use case |
| ID check (DigiLocker) | New port `IdentityVerifier`, flag on `users` |
| Paid verified listing | New `payments` use case, flag on `profiles` |
| Photos | Object storage port, `profile_media` table |
| Area search by name | `Geocoder` port |
| Message broker | Replace the outbox dispatcher only |

## 13. Operations

- **Deploy:** Inframan, from this repo's `inframan.yaml`. PostgreSQL is
  declared in the file and owned by the app.
- **bot:** `type: worker`, fixed 1 replica, recreate rollout, a 1 GiB disk for
  the WhatsApp session file. Two bots on one number would both answer.
- **api:** `type: web`, port 8080, domain `rentlo.inframan.live`.
- **Migrations:** embedded in the binary. `bot` applies them at start, under
  an advisory lock.
- **Logs:** JSON lines to stdout. Phone numbers are never logged; user ids are.
- **Health:** `/readyz` checks the database. The bot logs a heartbeat and the
  WhatsApp link state.
- **Limits:** at most 3 match alerts per user per day. No alerts from 22:00 to
  08:00 India time. 20 inbound messages per user per minute, then silence.

## 14. Risks, stated plainly

1. **The WhatsApp client is unofficial.** WhatsApp may ban the number, most of
   all if the bot messages people first or volume grows fast. The bot only
   answers people who wrote to it, and alerts are capped. The fix for real
   scale is the official Cloud API, which needs Meta business verification.
   Start that process now if the product gets traction.
2. **Shared number.** The owner's bot number also runs the Smash Karts bot.
   Rentlo answers direct chats only and ignores groups. A ban hits both.
3. **Cold start.** A match needs two people in one city. The map shows city
   counts so an empty area still shows activity elsewhere.
4. **Spot nodes.** A preemption restarts the bot. The session file is on a
   disk and survives. Messages sent while it is down arrive on reconnect.
5. **Personal data.** Phone numbers and home areas fall under India's DPDP
   Act. Consent is recorded at first message. `delete` works at once.

## 15. Build phases

Each phase ends with passing tests, a commit and a push.

| Phase | Content | Done when |
|---|---|---|
| 1 | Workspace, `domain`: types, validation, matching, blur, conversation machine | Unit tests pass |
| 2 | `store`: schema, repositories, outbox. `app`: use cases | Tests pass against a real PostgreSQL |
| 3 | `llm`: rule extractor and LLM client. Bot flow end to end with a fake messenger | Scripted chat test passes |
| 4 | `rentlo api`: routes, SSE, sessions | HTTP tests pass |
| 5 | `web`: map PWA | Builds, loads against the local API |
| 6 | `wa`: WhatsApp adapter, `rentlo bot` | Compiles, starts, prints a link code |
| 7 | Dockerfile, `inframan.yaml`, CI, deploy | Live at `rentlo.inframan.live` |

### Steps only the owner can do (phase 7)

1. Give the Inframan GitHub App (`inframan-git`) access to the `rentlo` repo.
2. Add a DNS `A` record: `rentlo.inframan.live` → `34.93.175.240`.
3. Enter the link code from the bot's log on the bot phone.

## 16. Engineering rules

- `domain` has no I/O. If a function needs the time, it takes it as an argument.
- Illegal states do not compile: enums over strings, newtypes over raw ids,
  validated constructors over public fields.
- No `unwrap` or `expect` outside tests and startup wiring.
- Every use case is one transaction. State and its outbox rows commit together.
- Every handler is idempotent. Every external call has a timeout.
- Tests: pure unit tests in `domain`, real-database tests in `store`, a
  scripted conversation test for the bot, HTTP tests for the API.
- `cargo fmt`, `cargo clippy -D warnings`, `tsc --noEmit` gate every commit.
- Comments explain why, never what.
