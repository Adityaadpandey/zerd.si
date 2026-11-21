# Hidden test to write
- Book a slot starting 1 hour ago -> 400 with a message that mentions the past.
- Book a slot overlapping an existing seeded booking (same venue, same resource) -> 400.
- Book a valid future, non-overlapping slot -> 2xx.
- Adjacent slots (one ends 18:00, next starts 18:00) -> both allowed.
