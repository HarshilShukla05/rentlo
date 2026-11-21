# Rentlo

Find a flatmate who lives like you, on WhatsApp. Send "Hi", describe
yourself in plain words, get matched. The map opens with a private WhatsApp link and shows approximate areas and match signals.

- Decisions: [`docs/PLAN.md`](docs/PLAN.md)
- Current map flow and local preview. [`docs/map-flow.md`](docs/map-flow.md)
- Original implementation handoff. [`docs/HANDOFF.md`](docs/HANDOFF.md)

```bash
export RENTLO_TEST_DATABASE_URL=postgres://postgres@localhost:5432/postgres
cargo test --workspace
```
