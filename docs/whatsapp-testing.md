# WhatsApp location and release tests

Pull requests run Rust tests and conversation tests with a fresh PostgreSQL
database and a recorded WhatsApp outbox. The location test checks the native
message shapes, coordinate and name-only Google links, persistence, and that a
second live-location message does not move the pin. The existing conversation
suite covers two synthetic users, matching, mutual consent, and erasure.

## Live WhatsApp check

Use two project-owned WhatsApp test accounts: one for the QA bot and one for
the automated tester. Their messages use synthetic profiles and public test
coordinates. The test runs the exact candidate commit as a local bot with a
fresh PostgreSQL service. Personal accounts and the production database are
outside the flow.

Register a trusted Linux self-hosted runner with Docker and the
`rentlo-whatsapp-qa` label. Keep the two WhatsApp SQLite files outside all Git
checkouts and restrict them to the runner account. Pair each account once from
a private terminal. Use the following command for each one, pointing it at the
appropriate session path and exporting the corresponding QA phone number:

```sh
cargo run --locked -p wa --example live_flow -- pair
```

The command reads `RENTLO_QA_TESTER_SESSION` for the path and
`RENTLO_QA_TESTER_PHONE` for the number during pairing. Both sessions use the
same format. For the live workflow, add these settings to the
`rentlo-whatsapp-qa` GitHub Actions environment:

- `RENTLO_QA_BOT_SESSION` and `RENTLO_QA_TESTER_SESSION` environment variables.
  Each points to a different paired SQLite file on the QA runner.
- `RENTLO_QA_BOT_PHONE` and `RENTLO_QA_TESTER_PHONE` environment secrets.
  Use their international numbers in digits. The workflow verifies both locally
  stored account identities before connecting.

Push a lightweight tag named `rentlo-test-<label>` on the candidate commit to
run the check. Once the workflow is on the default branch, you can also run it
from **Actions → WhatsApp release check → Run workflow** on
`rentlo-release-candidate`. The job summary and step log list each assertion
with a PASS or FAIL. Message payloads, account numbers, pairing codes, and
private links are omitted from the logs. A missing account, a lost connection,
or a missing expected reply fails the check.

## Promotion

To deploy from the same run, push a lightweight tag named
`rentlo-release-<label>` on the candidate commit, or select
**Deploy the tested commit to Inframan** when running the workflow manually.
The release job requires the live check to pass and only fast-forwards the
deployment branch to the commit that passed. Inframan tracks that branch.
The GitHub run shows when the commit was promoted, and Inframan Runs shows
the rollout status.

Set `RENTLO_RELEASE_TOKEN` in the `rentlo-production` Actions environment to a
release credential with permission to push to the deployment branch. Limit
access to the QA runner and release credential to trusted maintainers. This
workflow gates its own promotion; repository branch rules determine who can
deploy by pushing or merging outside the workflow.

## Former production session

The production manifest selects a fresh WhatsApp SQLite location. Startup
removes the two former WhatsApp session files and their SQLite sidecars from
the bot volume. Inframan logs the number of files removed. This leaves the
application PostgreSQL data and the replacement session in place. Set the
replacement pairing number in the existing Inframan secret before activating
the new bot revision. The workflow's QA numbers are separate from this secret.
