#!/bin/sh
# Hidden acceptance test. The runner calls it with the fixed worktree as the
# current directory, so "from textutil import ..." finds the agent's code.
python3 -c "
from textutil import shout
assert shout('hi') == 'HI!', shout('hi')
assert shout('') == '!', shout('')
print('hidden ok')
"
