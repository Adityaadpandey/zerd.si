# Hidden test to write
- A booking for 23:00 to 01:00 IST: the confirmation text/date fields show the next day's date for the end time.
- A booking for 05:00 IST (23:30 UTC the day before) shows the IST date, not the UTC date.
- Stored values in the DB are unchanged UTC timestamps.
