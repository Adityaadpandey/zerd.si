---
name: testing
description: Writing and fixing tests that actually catch bugs
triggers: test, tests, testing, coverage, regression test, unit test, spec, assertion, mock
---
# Testing
Follow the project's existing runner and layout; never introduce a second framework. A regression test must fail before the fix and pass after: run it both ways. Test behavior and edge cases (empty, boundary, error, concurrent), not implementation details. Do not weaken, skip or delete a test to get green; change an expectation only when the intended behavior genuinely changed, and say so. Prefer real objects over mocks where cheap, except at the network edge: UI and end-to-end tests that read a live API get a fixture or a route mock (Playwright `page.route`, MSW), so an outage cannot fail them and a run is repeatable. When a test fails because the environment is down, report it as an environment failure and keep the assertion as it was; never relax an assertion to tolerate an error state the app should not show.
