# Rentlo map onboarding

1. Start in WhatsApp. Offer a room, seek a room, or seek a room and stay
   open to searching with a flatmate. Capture the city, neighbourhood,
   budget and preferences in the same conversation. Team-up members can
   match both room offers and compatible people searching together.
2. After the profile is live, send one private map link. A tap opens the
   map and exchanges its single-use, 15-minute token on the server. The
   browser uses an HTTP-only session cookie. Members can request another
   link in WhatsApp at any time.
3. Centre the private map on their saved neighbourhood or an approximate
   grid cell from a shared pin. Known neighbourhood names and aliases
   need no pin. For a neighbourhood outside the launch catalogue, show
   the known city and keep the typed neighbourhood in the owner’s card.
   A WhatsApp area pin can refine the map at any time.
4. Show the strongest compatibility reasons, budget and broad area
   before asking for interest. Matching uses neighbourhood precision.
   A member can pause or go live with one tap. Contact is shared after
   mutual interest.

The public base map contains the city and neighbourhood catalogue. The
activity API requires a verified member session and returns bucketed
counts by neighbourhood and intent when at least three members qualify.
The private map draws this activity at neighbourhood centres. City switching,
intent filters, and the neighbourhood list are available under **Explore areas**.
There are no individual public pins or cards. The browser checks for
updates every 30 seconds while the member’s tab is visible. The member’s
own area and matches stay private.

## Run locally

Build the web assets in `web/` with `npm ci` and `npm run build`, then
run `cargo run -p rentlo -- api` from the repository root with
`DATABASE_URL` pointing to PostgreSQL and
`PUBLIC_URL=http://127.0.0.1:8080`. The API listens on port 8080 by default.
Set the bot’s `PUBLIC_URL` to the same origin for generated map links.
The synthetic PostgreSQL tests cover the handoff, area selection, private
match signals, and participation controls.

## Release

The Docker image includes the frontend and serves it from the `api`
service. Inframan assigns the service an HTTPS address with
`auto_domain: true`. The bot’s private links use that address. Keep the
browser origin and both services’ `PUBLIC_URL` identical. The existing pairing-number secret can be populated with the
replacement production account when it is time to connect WhatsApp.
