-- Each stage carries a color (a palette token like 'blue', not a raw hex, so it
-- re-themes in dark mode). Editable per stage in Settings. Defaults to 'gray';
-- existing stages are backfilled by position so current pipelines get distinct,
-- sensible colors matching the Full-cycle template order.
ALTER TABLE stages ADD COLUMN color TEXT NOT NULL DEFAULT 'gray';

UPDATE stages SET color = CASE (position % 8)
    WHEN 0 THEN 'blue'
    WHEN 1 THEN 'amber'
    WHEN 2 THEN 'green'
    WHEN 3 THEN 'purple'
    WHEN 4 THEN 'teal'
    WHEN 5 THEN 'pink'
    WHEN 6 THEN 'red'
    ELSE 'gray'
END;
