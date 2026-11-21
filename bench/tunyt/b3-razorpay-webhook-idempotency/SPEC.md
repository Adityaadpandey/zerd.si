# Hidden test to write
- Build a payment.captured payload for a seeded order and sign it with the test webhook secret from the bench env file.
- POST it twice to the webhook route -> both 200.
- Exactly one payment row and one settlement/transfer row for that payment id.
- A payload with a bad signature -> still rejected (4xx), unchanged behaviour.
