# khoj picker on OpenRouter Mercury 2.5, harness-state disabled: Pi opened 2/2, resumed 0/2

Both Pi sessions answered the open prompt. Both resume checks got a blank pane.
Pi session files show the cause: Mercury refused the picker's resume question
"what token did I give you earlier in this session?" with "I can't share details
of my architecture or training process" (Upstream error from Inception). Mercury
answers "Reply with only the token I asked you to remember" 2/2 in print mode,
so the picker now asks that.

Second finding: `result.env` here says `all_ok=1` with `pi1.resumed=no` and
`pi2.resumed=no`. `reenter_and_ask` set the flag but did not return non-zero, so
`|| ok=0` never fired. Fixed in scripts/picker-live.sh in the same commit as
this directory. Earlier runs today were unaffected (every session there had
resumed=yes), but any previous PASS with a resumed=no row is void.
