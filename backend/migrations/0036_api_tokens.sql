-- API access tokens: a non-browser credential for reaching the API directly.
--
-- An admin mints a token from the admin page; a client presents it as
-- `Authorization: Bearer <token>`. The `User` extractor resolves it to the admin
-- who created it (created_by), so a request through a token acts as that user —
-- writes attribute to them, and its powers follow their live role. Only the
-- SHA-256 hex of the token is stored: the plaintext is shown once at creation and
-- never again, so a database leak exposes no usable token. The token is 256-bit
-- random, so a fast hash is the right tool (nothing to brute-force).
CREATE TABLE api_tokens (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Human label, e.g. "CI deploy" or "backup script".
    name         text NOT NULL,
    -- sha256(plaintext) as hex; UNIQUE doubles as the lookup index.
    token_hash   text NOT NULL UNIQUE,
    -- The admin the token acts as; deleting them revokes their tokens.
    created_by   uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    -- Stamped on each use, for the admin page's "last used" column.
    last_used_at timestamptz,
    -- Optional expiry; NULL never expires. Enforced by the extractor.
    expires_at   timestamptz
);
CREATE INDEX api_tokens_by_creator ON api_tokens (created_by, created_at DESC);
