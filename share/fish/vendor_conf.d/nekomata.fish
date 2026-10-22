# Fish already marks prompts and user commands. Mark its startup configuration
# as well, so felis can safely discard and repaint the very first prompt.
status is-interactive; or return
set -q NEKOMATA_SHELL_INTEGRATION; or return
status test-feature mark-prompt; or return

printf '\e]133;C\a'
function __nekomata_end_startup --on-event fish_prompt
    set -l startup_status $status
    printf '\e]133;D;%s\a' $startup_status
    functions --erase __nekomata_end_startup
end
