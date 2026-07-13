#!/usr/bin/env fish
# ai-model-router.fish
#
# Deterministic, token-free task classification for GitHub Copilot CLI and
# OpenAI Codex CLI. It starts a NEW interactive CLI thread with the selected
# provider/model. It does not change the model of an already-open Zed ACP thread.
#
# Examples:
#   ./ai-model-router.fish --provider copilot "Fix the typo in README.md"
#   ./ai-model-router.fish --provider codex "Review the async Rust architecture"
#   ./ai-model-router.fish --provider auto "Harden authentication and add tests"
#   ./ai-model-router.fish --dry-run --provider codex "Refactor the parser"
#   ./ai-model-router.fish --install
#
# Optional environment overrides:
#   AI_ROUTER_COPILOT_SIMPLE
#   AI_ROUTER_COPILOT_STANDARD
#   AI_ROUTER_COPILOT_COMPLEX
#   AI_ROUTER_COPILOT_CRITICAL
#   AI_ROUTER_CODEX_SIMPLE
#   AI_ROUTER_CODEX_STANDARD
#   AI_ROUTER_CODEX_COMPLEX
#   AI_ROUTER_CODEX_CRITICAL
#
# Copilot strategy:
#   native  -> uses Copilot's own task-aware "auto" model router
#   pinned  -> uses the model map below
#
# The script deliberately performs no AI call for classification, so routing
# itself consumes no Copilot AI credits and no Codex quota.

function _air_info
    printf '\033[1;34m[INFO]\033[0m %s\n' "$argv"
end

function _air_ok
    printf '\033[1;32m[OK]\033[0m %s\n' "$argv"
end

function _air_warn
    printf '\033[1;33m[WARN]\033[0m %s\n' "$argv" >&2
end

function _air_die
    printf '\033[1;31m[ERROR]\033[0m %s\n' "$argv" >&2
    exit 1
end

function _air_usage
    echo "Usage:"
    echo "  ai-model-router.fish [OPTIONS] TASK..."
    echo "  ai-model-router.fish --install"
    echo
    echo "Options:"
    echo "  -p, --provider PROVIDER  copilot, codex, agy, or auto (default: auto)"
    echo "  -s, --strategy STRATEGY  Copilot: native or pinned (default: native)"
    echo "  -l, --level LEVEL        Force simple, standard, complex, or critical"
    echo "  -d, --dry-run            Show selection and command without starting it"
    echo "  -y, --yes                Start without confirmation"
    echo "      --install            Install as ~/.local/bin/ai-route"
    echo "  -h, --help               Show this help"
    echo
    echo "Automatic provider policy:"
    echo "  simple/standard -> Copilot"
    echo "  complex         -> Codex"
    echo "  critical/agent  -> Agy"
end

argparse \
    'h/help' \
    'p/provider=' \
    's/strategy=' \
    'l/level=' \
    'd/dry-run' \
    'y/yes' \
    'install' \
    -- $argv
or begin
    _air_usage
    exit 2
end

if set -q _flag_help
    _air_usage
    exit 0
end

if set -q _flag_install
    set -l source_file (status --current-filename)
    set -l target_dir "$HOME/.local/bin"
    set -l target "$target_dir/ai-route"

    test -f "$source_file"; or _air_die "Die aktuelle Skriptdatei wurde nicht gefunden."

    mkdir -p "$target_dir"
    cp "$source_file" "$target"
    chmod 755 "$target"

    if not contains -- "$target_dir" $PATH
        fish_add_path -U "$target_dir"
        _air_info "$target_dir wurde dem universellen fish-PATH hinzugefügt."
    end

    _air_ok "Installiert: $target"
    echo "Aufruf: ai-route --provider codex \"Deine Aufgabe\""
    exit 0
end

set -l provider auto
if set -q _flag_provider
    set provider (string lower -- "$_flag_provider")
end

if not contains -- "$provider" copilot codex agy auto
    _air_die "Ungültiger Provider: $provider"
end

set -l strategy native
if set -q _flag_strategy
    set strategy (string lower -- "$_flag_strategy")
end

if not contains -- "$strategy" native pinned
    _air_die "Ungültige Strategie: $strategy"
end

set -l task (string trim -- (string join ' ' -- $argv))
if test -z "$task"
    read -l -P "Aufgabe: " task
    set task (string trim -- "$task")
end

test -n "$task"; or _air_die "Keine Aufgabe angegeben."

# Model defaults. Override any value with the matching environment variable.
set -q AI_ROUTER_COPILOT_SIMPLE;   or set AI_ROUTER_COPILOT_SIMPLE   "gpt-5-mini"
set -q AI_ROUTER_COPILOT_STANDARD; or set AI_ROUTER_COPILOT_STANDARD "gpt-5.6-luna"
set -q AI_ROUTER_COPILOT_COMPLEX;  or set AI_ROUTER_COPILOT_COMPLEX  "gpt-5.6-terra"
set -q AI_ROUTER_COPILOT_CRITICAL; or set AI_ROUTER_COPILOT_CRITICAL "gpt-5.6-sol"

set -q AI_ROUTER_CODEX_SIMPLE;   or set AI_ROUTER_CODEX_SIMPLE   "gpt-5.4-mini"
set -q AI_ROUTER_CODEX_STANDARD; or set AI_ROUTER_CODEX_STANDARD "gpt-5.6-luna"
set -q AI_ROUTER_CODEX_COMPLEX;  or set AI_ROUTER_CODEX_COMPLEX  "gpt-5.6-terra"
set -q AI_ROUTER_CODEX_CRITICAL; or set AI_ROUTER_CODEX_CRITICAL "gpt-5.6-sol"

set -q AI_ROUTER_AGY_SIMPLE;   or set AI_ROUTER_AGY_SIMPLE   "Gemini 3.5 Flash (Low)"
set -q AI_ROUTER_AGY_STANDARD; or set AI_ROUTER_AGY_STANDARD "Gemini 3.5 Flash (Medium)"
set -q AI_ROUTER_AGY_COMPLEX;  or set AI_ROUTER_AGY_COMPLEX  "Claude Sonnet 4.6 (Thinking)"
set -q AI_ROUTER_AGY_CRITICAL; or set AI_ROUTER_AGY_CRITICAL "Claude Opus 4.6 (Thinking)"

set -l score 0
set -l reasons
set -l task_lc (string lower -- "$task")
set -l task_length (string length -- "$task")

# Scope and ambiguity.
if test "$task_length" -gt 900
    set score (math "$score + 3")
    set -a reasons "+3 very long task description"
else if test "$task_length" -gt 350
    set score (math "$score + 2")
    set -a reasons "+2 long task description"
else if test "$task_length" -gt 140
    set score (math "$score + 1")
    set -a reasons "+1 moderately detailed task"
end

# High-risk and high-judgment work.
if string match -rq -- \
    '(security|sicherhe|vulnerab|exploit|authent|authori[sz]|crypto|verschlüssel|encryption|secret|credential|firmware|kernel|supply.?chain|incident|datenverlust|data.?loss|production|produktio)' \
    "$task_lc"
    set score (math "$score + 4")
    set -a reasons "+4 security, production, or irreversible-risk terms"
end

# Architectural and difficult engineering work.
if string match -rq -- \
    '(architect|architektur|migration|distributed|concurr|parallel|async|race condition|deadlock|unsafe|memory safety|performance|profiling|database|schema|protocol|reverse engineer|refactor.*multiple|mehrere datei|multi.?file)' \
    "$task_lc"
    set score (math "$score + 3")
    set -a reasons "+3 architecture, concurrency, migration, or multi-file terms"
end

# Normal implementation work.
if string match -rq -- \
    '(implement|feature|refactor|debug|bug|fix|test|api|cli|gui|tui|integration|pipeline|workflow|dependency|upgrade|review|analyse|analysiere)' \
    "$task_lc"
    set score (math "$score + 2")
    set -a reasons "+2 implementation, debugging, testing, or review terms"
end

# Explicitly small, mechanical tasks.
if string match -rq -- \
    '(typo|rechtschreib|rename|umbenenn|format|formatier|kommentar|comment|readme|documentation only|nur dokumentation|one line|eine zeile|single file|eine datei)' \
    "$task_lc"
    set score (math "$score - 2")
    set -a reasons "-2 mechanical or narrowly scoped terms"
end

# Use a small amount of repository context without sending any content to AI.
if command -q git; and git rev-parse --is-inside-work-tree >/dev/null 2>&1
    set -l tracked_files (git ls-files 2>/dev/null | wc -l | string trim)
    set -l changed_files (git status --short 2>/dev/null | wc -l | string trim)

    if string match -rq '^[0-9]+$' -- "$tracked_files"
        if test "$tracked_files" -gt 2000
            set score (math "$score + 2")
            set -a reasons "+2 repository has more than 2000 tracked files"
        else if test "$tracked_files" -gt 500
            set score (math "$score + 1")
            set -a reasons "+1 repository has more than 500 tracked files"
        end
    end

    if string match -rq '^[0-9]+$' -- "$changed_files"; and test "$changed_files" -gt 20
        set score (math "$score + 2")
        set -a reasons "+2 worktree has more than 20 changed files"
    else if string match -rq '^[0-9]+$' -- "$changed_files"; and test "$changed_files" -gt 5
        set score (math "$score + 1")
        set -a reasons "+1 worktree has more than 5 changed files"
    end
end

set -l level
if set -q _flag_level
    set level (string lower -- "$_flag_level")
    if not contains -- "$level" simple standard complex critical
        _air_die "Ungültiges Level: $level"
    end
    set -a reasons "manual level override"
else if test "$score" -le 1
    set level simple
else if test "$score" -le 4
    set level standard
else if test "$score" -le 7
    set level complex
else
    set level critical
end

# Balanced automatic provider policy.
if test "$provider" = auto
    if string match -rq -- '(agent|agy|workspace|audit|compliance|refactor.*multiple|pkgbuild|packaging|systemd)' "$task_lc"
        set provider agy
    else
        switch "$level"
            case simple standard
                set provider copilot
            case complex
                set provider codex
            case critical
                set provider agy
        end
    end
end

set -l model
set -l effort
set -l command_line

switch "$provider"
    case copilot
        command -q copilot; or _air_die "copilot wurde nicht gefunden."

        if test "$strategy" = native
            # GitHub's own task-aware model routing.
            set model auto
        else
            switch "$level"
                case simple
                    set model "$AI_ROUTER_COPILOT_SIMPLE"
                case standard
                    set model "$AI_ROUTER_COPILOT_STANDARD"
                case complex
                    set model "$AI_ROUTER_COPILOT_COMPLEX"
                case critical
                    set model "$AI_ROUTER_COPILOT_CRITICAL"
            end
        end

        set command_line copilot "--model=$model" -i "$task"

    case codex
        command -q codex; or _air_die "codex wurde nicht gefunden."

        switch "$level"
            case simple
                set model "$AI_ROUTER_CODEX_SIMPLE"
                set effort low
            case standard
                set model "$AI_ROUTER_CODEX_STANDARD"
                set effort medium
            case complex
                set model "$AI_ROUTER_CODEX_COMPLEX"
                set effort high
            case critical
                set model "$AI_ROUTER_CODEX_CRITICAL"
                set effort xhigh
        end

        set -l effort_override "model_reasoning_effort=\"$effort\""
        set command_line codex --model "$model" --config "$effort_override" "$task"

    case agy
        command -q agy; or _air_die "agy wurde nicht gefunden."

        switch "$level"
            case simple
                set model "$AI_ROUTER_AGY_SIMPLE"
            case standard
                set model "$AI_ROUTER_AGY_STANDARD"
            case complex
                set model "$AI_ROUTER_AGY_COMPLEX"
            case critical
                set model "$AI_ROUTER_AGY_CRITICAL"
        end

        # Agent selection logic based on keywords
        set -l agent_name "self"
        if string match -rq -- '(research|search|find|lookup|query|info|read.?only)' "$task_lc"
            set agent_name "research"
            set -a reasons "routed to 'research' agent due to search/lookup terms"
        end

        set command_line agy --model "$model" --agent "$agent_name" -i "$task"
end

echo
echo "Task routing"
echo "------------"
printf "Provider:  %s\n" "$provider"
printf "Level:     %s\n" "$level"
printf "Score:     %s\n" "$score"
printf "Model:     %s\n" "$model"

if test "$provider" = codex
    printf "Reasoning: %s\n" "$effort"
else if test "$provider" = copilot
    printf "Strategy:  %s\n" "$strategy"
end

if test (count $reasons) -gt 0
    echo "Reasons:"
    for reason in $reasons
        echo "  - $reason"
    end
end

echo
echo "Command:"
string join ' ' -- (string escape -- $command_line)
echo

if set -q _flag_dry_run
    exit 0
end

if not set -q _flag_yes
    read -l -P "Neuen Thread starten? [J/n] " answer
    set answer (string lower -- (string trim -- "$answer"))

    if contains -- "$answer" n nein no
        _air_warn "Abgebrochen."
        exit 0
    end
end

command $command_line
