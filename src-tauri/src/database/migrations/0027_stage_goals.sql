-- v27 — a stage is no longer just a label on a column: it carries the GOAL of
-- that step and the pace it's meant to move at.
--
-- `goal` is the objective of the step itself — what has to become true before a
-- prospect belongs in the next column. Two things read it:
--   - the draft composer, which now knows where the thread SITS (this goal) as
--     well as where the buyer is headed (`customers.goal`), and
--   - the advance analyzer, which after each new message asks whether this goal
--     has been met and proposes a move.
--
-- This is a deliberate partial reversal of 0023's "the goal is not a pointer at
-- a pipeline stage". That still holds for `customers.goal` — the buyer's
-- destination, which spans the whole funnel. What was missing is the other
-- coordinate: a per-step objective concrete enough to have an exit condition.
-- The two coexist; neither replaces the other.
--
-- `warn_days` / `stale_days` are that step's tolerance for silence, in days
-- since your last outreach (see 0028). Per-stage rather than global because a
-- freshly-messaged prospect rots faster than one mid-onboarding. Defaults match
-- the app's built-in cadence, so an untouched pipeline behaves sensibly and the
-- fields are an override, never a chore.
ALTER TABLE stages ADD COLUMN goal TEXT NOT NULL DEFAULT '';
ALTER TABLE stages ADD COLUMN warn_days INTEGER NOT NULL DEFAULT 3;
ALTER TABLE stages ADD COLUMN stale_days INTEGER NOT NULL DEFAULT 7;
