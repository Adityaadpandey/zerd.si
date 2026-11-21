# Hidden test to write
- Seed (or pick) a slot with exactly 1 spot left.
- Fire 25 concurrent booking requests from different users (Promise.all with fetch, or `xargs -P 25 curl`).
- Exactly 1 response is 2xx, the other 24 are 409.
- Query bookings for that slot afterwards: confirmed count == capacity.
- Run it 3 times in the test: a race fix that works 1 time in 3 is not a fix.
