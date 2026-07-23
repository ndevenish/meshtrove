-- A token may carry a role, so an admin can mint a read-only (viewer) or editor
-- token, not only a full-admin one. Existing tokens default to 'admin' (their
-- prior behaviour). The extractor resolves a request's role as the *least*
-- privilege of this column and the owner's live role, so a token never exceeds
-- its owner — a demoted owner drags their tokens down with them.
ALTER TABLE api_tokens ADD COLUMN role user_role NOT NULL DEFAULT 'admin';
