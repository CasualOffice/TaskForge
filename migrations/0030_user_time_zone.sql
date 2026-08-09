-- 0030 — A user has a time zone, and `@today` finally means their today.
--
-- `casual-task-search`'s resolver takes a `UtcOffset` and has **no default**,
-- deliberately: docs/27 §Symbolic values says "`due before @today` must mean the
-- same thing to someone in Auckland and someone in Los Angeles. Server-local
-- date boundaries are a classic and extremely confusing bug."
--
-- The API then passed `UtcOffset::UTC` at every call site, because nothing
-- stored a zone. So the type made the mistake impossible to write by accident
-- and the application made it anyway, in the one place the type could not see.
-- This column is what the offset is derived from.
--
-- # An IANA name, not an offset
--
-- `Australia/Sydney`, not `+11:00`. An offset cannot answer "what will midnight
-- be in three weeks" — it changes twice a year in most of the world — and a
-- stored offset would silently drift from the user's real day boundary every
-- time daylight saving moved. The name survives that.
--
-- **The offset is not derived from this name yet.** No time-zone database is a
-- dependency of this workspace, so evaluation uses the offset the client sends
-- with the request, which a browser computes correctly including daylight
-- saving. The name is the durable record and is what a server-side job — a
-- digest, a scheduled notification — will need when there is no client to ask.
-- Adding a tz database is D-065.
--
-- NULL means "not set", which is not the same as UTC. A user who has never
-- chosen one is a user whose day boundary we do not know, and the API says so
-- rather than guessing on their behalf.
ALTER TABLE user_account ADD COLUMN time_zone text;

COMMENT ON COLUMN user_account.time_zone IS
    'IANA zone name, e.g. Australia/Sydney. NULL means unset, which is not UTC. '
    'Used to resolve @today and friends at evaluation time (docs/27).';
