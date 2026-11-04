# WolOS: PowerShell in zsh, sourced by the WolOS zshrc when the Console module's pwsh is installed
# (wopkg recipe powershell, ~/.local/bin/pwsh). docs/modules/pwsh.md says the same in prose.
#   pw 'LINE' [ARGS]  runs one pwsh line inside a zsh pipeline: stdin lines (a pipe or a file) go
#                     into its first command, its output comes out as text, its exit status is
#                     the line's (/usr/share/wolos/pwsh/pw.ps1 does the pwsh side). Not `ps`: that
#                     is the process list every script and person expects.
#   ./x.ps1, x.ps1    an executable file with a #! line runs as any script does; without one,
#                     `wol script run` runs it (clean environment, never root, the audit log).
#   Tab               after `pw '` and `pwsh -Command`: cmdlets, then their parameters, from a list
#                     pwsh writes in the background (/usr/share/wolos/pwsh/commands.ps1), once a day.
#   Colours           the pwsh line inside `pw '...'` and `pwsh -c '...'` is coloured: commands,
#                     parameters, variables, strings, comments (styles pwsh-* below).
# Startup cost: a few function definitions and one file-age test; pwsh itself never starts here.

# The person's pwsh is in ~/.local/bin, which no system file puts on PATH: add it at the end, so
# `pwsh` and `#!/usr/bin/env pwsh` work from this shell and nothing on the system is shadowed.
[[ -x $HOME/.local/bin/pwsh ]] && (( ! ${path[(Ie)$HOME/.local/bin]} )) && path+=("$HOME/.local/bin")

_wolos_pw_share=${WOLOS_PWSH_SHARE:-/usr/share/wolos/pwsh}
_wolos_pw_cache=${XDG_CACHE_HOME:-$HOME/.cache}/wolos/pwsh/commands.tsv

pw() {
  emulate -L zsh
  if [[ -z ${1-} ]]; then
    print -ru2 -- "pw: give one pwsh line, for example: pw 'Get-Date'"
    return 2
  fi
  local feed=0
  [[ -p /dev/stdin || -f /dev/stdin ]] && feed=1
  WOLOS_PW_LINE=$1 WOLOS_PW_INPUT=$feed ${commands[pwsh]:-/usr/lib/wolos/pwsh} \
    -NoLogo -NoProfile -NonInteractive -File "$_wolos_pw_share/pw.ps1" "${@[2,-1]}"
}

# `x.ps1` or `./x.ps1` typed as a command (zsh suffix alias): the kernel runs a file that has a #!
# line and the x bit, as for any script; anything else goes to `wol script run`.
_wolos_ps1() {
  local file=$1 first=
  shift
  [[ -r $file ]] && IFS= read -r first < "$file"
  if [[ -x $file && $first == '#!'* ]]; then
    [[ $file == */* ]] || file=./$file
    command -- "$file" "$@"
  else
    wol script run -- "$file" "$@"
  fi
}
alias -s ps1=_wolos_ps1

# ---------------------------------------------------------------- the command list

# Writes the list when it is missing or a day old: in the background, at low priority, one
# terminal at a time (a lock directory that a crashed run leaves for at most ten minutes).
_wolos_pw_refresh() {
  emulate -L zsh
  setopt extended_glob
  local lock=$_wolos_pw_cache.lock exe=${commands[pwsh]:-$HOME/.local/bin/pwsh}
  [[ -x $exe && -r $_wolos_pw_share/commands.ps1 ]] || return 0
  [[ -n $_wolos_pw_cache(#qN.mh-24) ]] && return 0
  [[ -n $lock(#qN/mm-10) ]] && return 0
  mkdir -p -- "${_wolos_pw_cache:h}" && rm -rf -- "$lock" && mkdir -- "$lock" 2>/dev/null || return 0
  ( nice -n 10 "$exe" -NoLogo -NoProfile -NonInteractive -File "$_wolos_pw_share/commands.ps1" \
      "$_wolos_pw_cache" </dev/null >/dev/null 2>&1; rmdir -- "$lock" ) &!
}
_wolos_pw_refresh

# The list in memory: what each command is, its parameters, and its name by lower case (pwsh names
# are case-insensitive). Read again when the file changes.
typeset -gA _wolos_pw_what _wolos_pw_params _wolos_pw_name
typeset -g _wolos_pw_stamp= _wolos_pw_common=
_wolos_pw_load() {
  emulate -L zsh
  zmodload -F zsh/stat b:zstat 2>/dev/null
  local -a st
  zstat -A st +mtime -- "$_wolos_pw_cache" 2>/dev/null || return 1
  [[ $st[1] == $_wolos_pw_stamp ]] && return 0
  _wolos_pw_what=() _wolos_pw_params=() _wolos_pw_name=()
  local name what params
  while IFS=$'\t' read -r name what params; do
    [[ -n $name ]] || continue
    if [[ $name == '*' ]]; then
      _wolos_pw_common=$params
      continue
    fi
    _wolos_pw_what[$name]=$what
    _wolos_pw_params[$name]=$params
    _wolos_pw_name[${(L)name}]=$name
  done < "$_wolos_pw_cache"
  _wolos_pw_stamp=$st[1]
}

# ---------------------------------------------------------------- completion

# Completes the pwsh line in the current word: a command at the start of the line or after
# | ; ( { , else a parameter of that command after `-`, else a variable after `$`. Every match
# carries its own space (-S ' '): inside `pw '...'` zsh would otherwise close the quote.
# $1: the pwsh words before this one when the line is not quoted (`pwsh -Command Get-Item -Pa`).
_wolos_pw_code() {
  if ! _wolos_pw_load; then
    _wolos_pw_refresh
    _message 'pwsh is listing its commands; press Tab again in a moment'
    return 1
  fi
  local line=${1:+$1 }$PREFIX
  compset -P '*[[:space:]|;({}=,]'
  local before=${line%"$PREFIX"}
  local starts='[|;({]'
  local -a items=(${(z)${before##*$~starts}})
  local cmd=${items[1]-}
  cmd=${_wolos_pw_name[${(L)cmd}]:-$cmd}
  local n
  if [[ $PREFIX == -* ]]; then
    compset -P '-'
    items=(${=_wolos_pw_params[$cmd]})
    (( $#items )) && compadd -S ' ' -r '' -J pwsh-parameters -X "%F{${WOLOS_PROMPT_MUTED:-8}}parameters of $cmd%f" -- $items
    compadd -S ' ' -r '' -J pwsh-common -X "%F{${WOLOS_PROMPT_MUTED:-8}}common parameters%f" -- ${=_wolos_pw_common}
  elif [[ $PREFIX == '$'* ]]; then
    compset -P '$'
    compadd -S '' -J pwsh-variables -- _ args input PSItem true false null HOME PWD PID LASTEXITCODE Error \
      Matches PSScriptRoot PSStyle PSVersionTable Host env: ExecutionContext
  else
    items=()
    for n in ${(k)_wolos_pw_what}; do
      items+=("${n//:/\\:}:${_wolos_pw_what[$n]}")
    done
    _describe -t pwsh-commands 'pwsh command' items -S ' ' -r ''
  fi
}

# `pw`: the first argument is the line; the rest are files and words for $args.
_wolos_pw() {
  if (( CURRENT == 2 )); then
    _wolos_pw_code
  else
    _files
  fi
}

# `pwsh`: its options; after -Command (all the words after it) or -CommandWithArgs (the next
# word) the pwsh line; after -File a .ps1 file.
_wolos_pwsh() {
  local i w mode= from=0
  for (( i = 2; i < CURRENT; i++ )); do
    w=${(L)words[i]}
    case $w in
      -(c|co|com|comm|comma|comman|command)) mode=all from=i ;;
      -(cwa|commandwithargs)) (( i == CURRENT - 1 )) && mode=one || mode=files ;;
      -(f|fi|fil|file)) (( i == CURRENT - 1 )) && mode=script || mode=files ;;
    esac
    # What follows these is the line, the script or its arguments, never pwsh options.
    [[ -n $mode ]] && break
  done
  case $mode in
    all) _wolos_pw_code "${(j: :)${(@Q)words[from+1,CURRENT-1]}}" ;;
    one) _wolos_pw_code ;;
    script) _files -g '*.ps1(-.)' ;;
    files) _files ;;
    *)
      if [[ $PREFIX == -* ]]; then
        compadd -- -Command -CommandWithArgs -ConfigurationName -CustomPipeName -EncodedCommand \
          -ExecutionPolicy -File -Help -InputFormat -Interactive -Login -MTA -NoExit -NoLogo \
          -NonInteractive -NoProfile -NoProfileLoadTime -OutputFormat -SettingsFile -SSHServerMode \
          -STA -Version -WindowStyle -WorkingDirectory
      else
        _files -g '*.ps1(-.)'
      fi
      ;;
  esac
}
(( $+functions[compdef] )) && compdef _wolos_pw pw && compdef _wolos_pwsh pwsh

# ---------------------------------------------------------------- colours

# A highlighter for the syntax highlighting plugin (zshrc loads it after this file): the pwsh line
# of `pw '...'` and `pwsh -c '...'` gets the styles below on top of the plugin's own colour for a
# quoted argument. ~/.zshrc may change any of them (ZSH_HIGHLIGHT_STYLES[pwsh-command]=fg=blue).
typeset -gA ZSH_HIGHLIGHT_STYLES
: ${ZSH_HIGHLIGHT_STYLES[pwsh-command]:=fg=green}
: ${ZSH_HIGHLIGHT_STYLES[pwsh-keyword]:=fg=yellow,bold}
: ${ZSH_HIGHLIGHT_STYLES[pwsh-parameter]:=fg=cyan}
: ${ZSH_HIGHLIGHT_STYLES[pwsh-operator]:=fg=magenta}
: ${ZSH_HIGHLIGHT_STYLES[pwsh-variable]:=fg=magenta,bold}
: ${ZSH_HIGHLIGHT_STYLES[pwsh-string]:=fg=yellow}
: ${ZSH_HIGHLIGHT_STYLES[pwsh-type]:=fg=blue}
: ${ZSH_HIGHLIGHT_STYLES[pwsh-comment]:=fg=8}
typeset -ga ZSH_HIGHLIGHT_HIGHLIGHTERS
(( $#ZSH_HIGHLIGHT_HIGHLIGHTERS )) || ZSH_HIGHLIGHT_HIGHLIGHTERS=(main)
(( ${ZSH_HIGHLIGHT_HIGHLIGHTERS[(Ie)pwsh]} )) || ZSH_HIGHLIGHT_HIGHLIGHTERS+=(pwsh)

# One region: 0-based start, end (exclusive), style.
_wolos_pw_hl() {
  region_highlight+=("$1 $2 ${ZSH_HIGHLIGHT_STYLES[$3]}")
}

# Colours the pwsh text $2 that starts at offset $1 of BUFFER. A small scanner, not pwsh's parser:
# comments, variables, double-quoted strings, [types], -parameters and -operators, keywords, and
# commands (Verb-Noun, or a listed name where a command starts).
_wolos_pw_paint() {
  emulate -L zsh
  setopt extended_glob
  local -i base=$1 i=1 j n
  local s=$2 c word
  local start=1
  n=${#s}
  _wolos_pw_load 2>/dev/null
  while (( i <= n )); do
    c=${s[i]}
    case $c in
      ('#')
        if (( i == 1 )) || [[ ${s[i-1]} == [[:space:]] ]]; then
          j=${s[i,-1][(i)$'\n']}
          _wolos_pw_hl $(( base + i - 1 )) $(( base + i - 1 + j - 1 )) pwsh-comment
          (( i += j )); start=1; continue
        fi ;;
      ('$')
        j=i+1
        if [[ ${s[j]} == '{' ]]; then
          while (( j <= n )) && [[ ${s[j]} != '}' ]]; do (( j++ )); done
          (( j++ ))
        else
          while (( j <= n )) && [[ ${s[j]} == [[:alnum:]_:?] ]]; do (( j++ )); done
        fi
        (( j > i + 1 )) && _wolos_pw_hl $(( base + i - 1 )) $(( base + j - 1 )) pwsh-variable
        i=j; start=0; continue ;;
      ('"')
        j=i+1
        while (( j <= n )) && [[ ${s[j]} != '"' ]]; do
          [[ ${s[j]} == '`' ]] && (( j++ ))
          (( j++ ))
        done
        _wolos_pw_hl $(( base + i - 1 )) $(( base + j )) pwsh-string
        i=j+1; start=0; continue ;;
      ('[')
        if [[ ${s[i,-1]} == (#b)(\[[[:alpha:]][[:alnum:]_.]#(\[\])#\])* ]]; then
          _wolos_pw_hl $(( base + i - 1 )) $(( base + i - 1 + ${#match[1]} )) pwsh-type
          (( i += ${#match[1]} )); continue
        fi ;;
      ('|'|';'|'{'|'('|$'\n')
        start=1 ;;
      ('%'|'?')
        if (( start )); then
          _wolos_pw_hl $(( base + i - 1 )) $(( base + i )) pwsh-command
          start=0
        fi ;;
      ('-')
        if (( i == 1 )) || [[ ${s[i-1]} == [[:space:]\(] ]]; then
          j=i+1
          while (( j <= n )) && [[ ${s[j]} == [[:alnum:]_] ]]; do (( j++ )); done
          if (( j > i + 1 )); then
            word=${(L)s[i+1,j-1]}
            if [[ $word == (c|i|)(eq|ne|gt|ge|lt|le|like|notlike|match|notmatch|contains|notcontains|in|notin|replace|split) ||
                  $word == (join|and|or|not|xor|band|bor|bxor|bnot|shl|shr|is|isnot|as|f) ]]; then
              _wolos_pw_hl $(( base + i - 1 )) $(( base + j - 1 )) pwsh-operator
            else
              _wolos_pw_hl $(( base + i - 1 )) $(( base + j - 1 )) pwsh-parameter
            fi
            i=j; start=0; continue
          fi
        fi ;;
      ([[:alpha:]_])
        j=i
        while (( j <= n )) && [[ ${s[j]} == [[:alnum:]_.\-] ]]; do (( j++ )); done
        word=${s[i,j-1]}
        if [[ ${(L)word} == (if|elseif|else|foreach|for|while|do|until|switch|function|filter|param|begin|process|end|return|break|continue|throw|try|catch|finally|trap|exit|in) ]]; then
          _wolos_pw_hl $(( base + i - 1 )) $(( base + j - 1 )) pwsh-keyword
          start=1
        elif [[ $word == [[:alpha:]]##-[[:alpha:]][[:alnum:]]# ]] || { (( start )) && (( $+_wolos_pw_name[${(L)word}] )) }; then
          _wolos_pw_hl $(( base + i - 1 )) $(( base + j - 1 )) pwsh-command
          start=0
        else
          start=0
        fi
        i=j; continue ;;
      ([[:space:]]) ;;
      (*) start=0 ;;
    esac
    (( i++ ))
  done
}

_zsh_highlight_highlighter_pwsh_predicate() {
  [[ $WIDGET == zle-line-finish ]] || _zsh_highlight_buffer_modified
}

# Finds the pwsh lines: the word after `pw`, the word after `pwsh ... -c`/`-cwa`, every word after
# `pwsh ... -Command`. Word positions come from walking BUFFER with zsh's own splitting.
_zsh_highlight_highlighter_pwsh_paint() {
  emulate -L zsh
  [[ $BUFFER == *pw* ]] || return 0
  local -a ws
  ws=(${(z)BUFFER})
  local w rest skip lw state=cmd
  local -i pos=1 at
  for w in $ws; do
    rest=${BUFFER[pos,-1]}
    skip=${rest%%$w*}
    (( at = pos + ${#skip} ))
    (( pos = at + ${#w} ))
    lw=${(L)w}
    case $state:$w in
      (*:(\;|\||\|\||\&\&|\&|\(|\)|\{|\}|\|\&))
        state=cmd; continue ;;
    esac
    case $state in
      (cmd)
        if [[ $w == pw ]]; then state=pw
        elif [[ $w == (*/|)pwsh ]]; then state=pwsh
        elif [[ $w == (noglob|command|nocorrect|builtin|exec|time|sudo|*=*) ]]; then state=cmd
        else state=other
        fi ;;
      (pw|one|all)
        if [[ $w == \'* ]]; then
          _wolos_pw_paint $at "${${w#\'}%\'}"
        elif [[ $w == \"* ]]; then
          _wolos_pw_paint $at "${${w#\"}%\"}"
        else
          _wolos_pw_paint $(( at - 1 )) "$w"
        fi
        [[ $state == all ]] || state=other ;;
      (pwsh)
        case $lw in
          (-(c|co|com|comm|comma|comman|command))) state=all ;;
          (-(cwa|commandwithargs))) state=one ;;
          (-*) ;;
          (*) state=other ;;
        esac ;;
    esac
  done
}
