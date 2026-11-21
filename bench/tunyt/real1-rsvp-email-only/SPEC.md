# Hidden test to write (before any agent runs)

Against a running API (`sh "$LIB/with_server.sh" ...`, seeded local DB):
1. POST the RSVP endpoint for a seeded public event with only `{ email }` (plus answers to the host's custom questions if the event has any) -> 2xx, and the guest shows as RSVPed for that event.
2. The response body and headers contain NO token, session id, or Set-Cookie for a session.
3. No verification email/code row is created for that email.
4. An event WITH custom questions still requires and stores the answers (missing required answer -> 4xx).
5. Same email twice -> no duplicate RSVP.

Put it in hidden_test/test.sh and run `bench/duel --validate`: it must FAIL on today's code.
