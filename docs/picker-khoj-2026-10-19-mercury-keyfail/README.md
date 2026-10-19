# khoj picker run on a22e167d, Pi on inception/mercury-2: FAIL (config, not build)

Claude 1 and 2 replied and resumed. Pi 1 and 2 never replied. Cause: khoj's
`~/.pi/agent/models.json` resolves the inception key with
`security find-generic-password -s 'mercury-sab' -w`, a macOS keychain command
copied from the Mac; on Linux Pi reports "API key auth failed for provider
inception" and the ACP pane stays blank. `pi -p --model deepseek/deepseek-flash`
on the same host answered. Gap for grok-build: a Pi auth failure is invisible in
the pane. The clean leader proof is the DeepSeek rerun beside this directory.
