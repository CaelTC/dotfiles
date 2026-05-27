#!/bin/sh
input=$(cat)

user=$(whoami)
dir=$(echo "$input" | jq -r '.workspace.current_dir // .cwd // ""')
short_dir=$(basename "$dir")

model=$(echo "$input" | jq -r '.model.display_name // ""')

transcript=$(echo "$input" | jq -r '.transcript_path // empty')
ctx_str=""
if [ -n "$transcript" ] && [ -f "$transcript" ]; then
  tokens=$(jq -s '[.[] | select(.type=="assistant")] | if length == 0 then 0 else (last.message.usage | (.input_tokens // 0) + (.cache_creation_input_tokens // 0) + (.cache_read_input_tokens // 0)) end' "$transcript" 2>/dev/null)
  if [ -n "$tokens" ] && [ "$tokens" -gt 0 ] 2>/dev/null; then
    if [ "$tokens" -gt 999000 ]; then
      ctx_str=$(awk -v t="$tokens" 'BEGIN { printf " tokens:%.2fM", t/1000000 }')
    else
      ctx_str=$(printf " tokens:%sk" "$(( tokens / 1000 ))")
    fi
  fi
fi

git_branch=""
if [ -d "$dir/.git" ] || git -C "$dir" rev-parse --git-dir > /dev/null 2>&1; then
  branch=$(git -C "$dir" --no-optional-locks symbolic-ref --short HEAD 2>/dev/null)
  if [ -n "$branch" ]; then
    git_branch=" ($branch)"
  fi
fi

printf "%s ~/%s%s | %s%s" "$user" "$short_dir" "$git_branch" "$model" "$ctx_str"
