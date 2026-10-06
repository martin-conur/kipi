# side-tool notes

## Gotchas

- Some cameras write EXIF dates without a timezone; treat them as local time, never UTC.

## Where to look

- `src/exif.rs`: date extraction and fallbacks.
