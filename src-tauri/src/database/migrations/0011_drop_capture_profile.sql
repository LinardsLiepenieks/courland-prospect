-- v11 — drop the capture_profile table. The app no longer runs or manages a
-- dedicated Chrome, so there's no persisted "active profile" to track: the
-- profile list is read live from the user's real Chrome, and switching just
-- opens that profile. Dev-only table (never shipped); nothing references it.
DROP TABLE IF EXISTS capture_profile;
