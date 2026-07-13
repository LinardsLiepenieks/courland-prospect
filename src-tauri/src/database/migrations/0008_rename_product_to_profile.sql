-- v8 — rename the `product` singleton table to `profile`. The feature was
-- renamed (the tab now reads "Profile"); this keeps the schema name in step.
--
-- ALTER TABLE ... RENAME TO preserves the columns, the CHECK (id = 1) singleton
-- constraint, and the existing seeded row — so no data is touched or reseeded.
ALTER TABLE product RENAME TO profile;
