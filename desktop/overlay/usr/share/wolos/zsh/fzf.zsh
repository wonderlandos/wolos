# WolOS fzf pickers for zsh, sourced by the WolOS zshrc only when fzf is installed. Each one opens
# fzf below the prompt and puts what you pick into the command line, quoted:
#   Ctrl+Alt+F  files under the directory you are typing (or here), with a preview
#   Ctrl+Alt+L  commits of this git repository; inserts full hashes
#   Ctrl+Alt+V  shell variables; inserts $name
# Defining them costs nothing at startup; fd, bat and git are used when installed.

_wolos_fzf_files() {
  emulate -L zsh
  local word=${LBUFFER##* } root=.
  # The word before the cursor names a directory: search there and replace the word.
  [[ -n $word && -d ${word/#\~/$HOME} ]] && root=${word/#\~/$HOME}
  local preview='head -n 100 -- {}'
  (( $+commands[bat] )) && preview='bat --color=always --style=plain --line-range=:100 -- {}'
  local -a list picked
  if (( $+commands[fd] )); then
    list=(fd --type f --hidden --exclude .git . "$root")
  else
    list=(find "$root" -type f -not -path '*/.git/*')
  fi
  picked=(${(f)"$("${list[@]}" 2>/dev/null | fzf --multi --height=40% --reverse --preview="$preview")"})
  if (( $#picked )); then
    [[ $root != . ]] && LBUFFER=${LBUFFER%"$word"}
    LBUFFER+="${(j: :)${(@q)picked}} "
  fi
  zle reset-prompt
}

_wolos_fzf_gitlog() {
  emulate -L zsh
  git rev-parse --git-dir >/dev/null 2>&1 || return 0
  local -a picked
  # Shown: short hash, subject, age. Inserted: the first field, the full hash.
  picked=(${(f)"$(git log --format='%H %h %s (%cr)' 2>/dev/null |
    fzf --multi --no-sort --height=40% --reverse --with-nth=2.. \
      --preview='git show --color=always --stat {1}' | cut -d' ' -f1)"})
  (( $#picked )) && LBUFFER+="${(j: :)picked} "
  zle reset-prompt
}

_wolos_fzf_vars() {
  emulate -L zsh
  local -a rows picked
  local k
  for k in ${(ko)parameters}; do
    rows+=("$k ${${(j: :)${(P)k}}[1,60]}")
  done
  picked=(${(f)"$(print -rl -- "${rows[@]}" | fzf --multi --height=40% --reverse | cut -d' ' -f1)"})
  (( $#picked )) && LBUFFER+="${(j: :)${(@)picked/#/\$}} "
  zle reset-prompt
}

zle -N _wolos_fzf_files
zle -N _wolos_fzf_gitlog
zle -N _wolos_fzf_vars
bindkey '^[^F' _wolos_fzf_files
bindkey '^[^L' _wolos_fzf_gitlog
bindkey '^[^V' _wolos_fzf_vars
