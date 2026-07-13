-- Distinguish who sent a captured message. Existing rows are all outgoing
-- (that's all we captured before), so the default keeps the derived
-- `messages_sent` count unchanged after the upgrade.
ALTER TABLE messages ADD COLUMN direction TEXT NOT NULL DEFAULT 'outgoing';

-- Durable engagement flag: has this prospect ever replied to us? Derived from
-- stored incoming messages (see features::messages), recomputed alongside
-- messages_sent. Defaults to 0; reality fills in as their threads are viewed.
ALTER TABLE prospects ADD COLUMN responded INTEGER NOT NULL DEFAULT 0;
