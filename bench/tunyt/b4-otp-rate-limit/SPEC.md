# Hidden test to write
- 5 OTP requests for one identifier -> all 2xx; the 6th -> 429 with a numeric Retry-After header.
- A different identifier from the same IP still works until the IP limit.
- package.json files unchanged (no new dependency): check with git diff.
- Clear the rate-limit keys in local Redis between test cases, through the app's own Redis client, not redis-cli.
