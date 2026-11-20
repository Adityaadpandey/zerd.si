#!/bin/sh
# The "existing suite" of the toy repo. It has to stay green after any fix.
cd "$(dirname "$0")/.." && python3 -c "
from textutil import whisper
assert whisper('HI') == 'hi'
print('suite ok')
"
