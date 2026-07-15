-- Give every existing model and bundle slug the random token that new ones now
-- carry (see routes/models.rs `unique_slug`). Uniqueness stops depending on
-- creation order — the first "Gold Warrior" no longer keeps the plain slug while
-- the second is stuck with "-2" — and a slug gains a stable identity that a
-- rename preserves.
--
-- Safe to append blindly: existing slugs are already unique, and adding a
-- fixed-length "-<5 hex>" segment to distinct strings keeps them distinct (two
-- results can only collide if their bases were equal, which they were not). The
-- token is derived per row, so the app's `slug_token_of` splits it back off on
-- the next rename.
UPDATE models  SET slug = slug || '-' || substr(md5(random()::text || id::text), 1, 5);
UPDATE bundles SET slug = slug || '-' || substr(md5(random()::text || id::text), 1, 5);
