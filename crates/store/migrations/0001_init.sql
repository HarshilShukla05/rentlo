-- Rentlo's schema. Every person-owned row hangs off users with ON DELETE
-- CASCADE, so erasing a user erases everything about them in one statement.

CREATE TABLE users (
    id          uuid PRIMARY KEY,
    jid         text NOT NULL UNIQUE,
    phone       text,
    first_name  text,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL
);

CREATE TABLE profiles (
    id           uuid PRIMARY KEY,
    user_id      uuid NOT NULL UNIQUE REFERENCES users ON DELETE CASCADE,
    code         text NOT NULL UNIQUE,
    status       text NOT NULL CHECK (status IN ('active', 'paused')),
    intent       text NOT NULL,
    city         text NOT NULL,
    area         text,
    lat          double precision,
    lng          double precision,
    budget_min   integer NOT NULL CHECK (budget_min >= 0),
    budget_max   integer NOT NULL CHECK (budget_max >= budget_min),
    move_in      date,
    gender       text NOT NULL,
    gender_pref  text NOT NULL,
    lifestyle    jsonb NOT NULL DEFAULT '{}',
    note         text,
    version      integer NOT NULL DEFAULT 1,
    created_at   timestamptz NOT NULL,
    updated_at   timestamptz NOT NULL,
    CHECK ((lat IS NULL) = (lng IS NULL))
);
CREATE INDEX profiles_matching ON profiles (city, updated_at DESC) WHERE status = 'active';
CREATE INDEX profiles_map ON profiles (lat, lng) WHERE status = 'active' AND lat IS NOT NULL;

CREATE TABLE conversations (
    user_id     uuid PRIMARY KEY REFERENCES users ON DELETE CASCADE,
    state       jsonb NOT NULL,
    updated_at  timestamptz NOT NULL
);

CREATE TABLE inbound_messages (
    message_id   text PRIMARY KEY,
    user_id      uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    received_at  timestamptz NOT NULL
);
CREATE INDEX inbound_by_user ON inbound_messages (user_id, received_at);

CREATE TABLE matches (
    id          uuid PRIMARY KEY,
    profile_a   uuid NOT NULL REFERENCES profiles ON DELETE CASCADE,
    profile_b   uuid NOT NULL REFERENCES profiles ON DELETE CASCADE,
    score       smallint NOT NULL CHECK (score BETWEEN 0 AND 100),
    decision_a  text CHECK (decision_a IN ('yes', 'skip')),
    decision_b  text CHECK (decision_b IN ('yes', 'skip')),
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL,
    UNIQUE (profile_a, profile_b),
    CHECK (profile_a < profile_b)
);
CREATE INDEX matches_by_b ON matches (profile_b);

CREATE TABLE outbox (
    id            bigserial PRIMARY KEY,
    topic         text NOT NULL,
    dedupe_key    text NOT NULL,
    user_id       uuid REFERENCES users ON DELETE CASCADE,
    jid           text NOT NULL,
    body          text NOT NULL,
    attempts      integer NOT NULL DEFAULT 0,
    available_at  timestamptz NOT NULL,
    created_at    timestamptz NOT NULL,
    sent_at       timestamptz,
    failed_at     timestamptz,
    last_error    text,
    UNIQUE (topic, dedupe_key)
);
CREATE INDEX outbox_due ON outbox (available_at, id) WHERE sent_at IS NULL AND failed_at IS NULL;
CREATE INDEX outbox_alerts ON outbox (user_id, created_at) WHERE topic = 'match.alert';

CREATE TABLE magic_links (
    token_hash  bytea PRIMARY KEY,
    user_id     uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    expires_at  timestamptz NOT NULL,
    used_at     timestamptz
);

CREATE TABLE web_sessions (
    token_hash  bytea PRIMARY KEY,
    user_id     uuid NOT NULL REFERENCES users ON DELETE CASCADE,
    expires_at  timestamptz NOT NULL
);

-- Reports outlive both people: they are the record of what happened.
CREATE TABLE reports (
    id           bigserial PRIMARY KEY,
    reporter_id  uuid REFERENCES users ON DELETE SET NULL,
    profile_id   uuid REFERENCES profiles ON DELETE SET NULL,
    reason       text NOT NULL,
    created_at   timestamptz NOT NULL
);

-- Wake-ups. A notification is only a hint to look; the rows are the truth,
-- so a missed one costs a poll interval, never a message.

CREATE FUNCTION notify_outbox() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('outbox', '');
    RETURN NULL;
END $$;
CREATE TRIGGER outbox_inserted AFTER INSERT ON outbox
    FOR EACH STATEMENT EXECUTE FUNCTION notify_outbox();

-- The map listens for pin changes. The payload is the public code only.
CREATE FUNCTION notify_pin() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM pg_notify('pins', OLD.code);
    ELSE
        PERFORM pg_notify('pins', NEW.code);
    END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER profiles_changed AFTER INSERT OR UPDATE OR DELETE ON profiles
    FOR EACH ROW EXECUTE FUNCTION notify_pin();
