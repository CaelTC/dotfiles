#!/bin/sh
input=$(cat)

user=$(whoami)
dir=$(echo "$input" | jq -r '.workspace.current_dir // .cwd // ""')
short_dir=$(basename "$dir")

model=$(echo "$input" | jq -r '.model.display_name // ""')

used=$(echo "$input" | jq -r '.context_window.used_percentage // empty')
if [ -n "$used" ]; then
  ctx_str=$(printf " ctx:%.0f%%" "$used")
else
  ctx_str=""
fi

git_branch=""
if [ -d "$dir/.git" ] || git -C "$dir" rev-parse --git-dir > /dev/null 2>&1; then
  branch=$(git -C "$dir" symbolic-ref --short HEAD 2>/dev/null)
  if [ -n "$branch" ]; then
    git_branch=" ($branch)"
  fi
fi

printf "%s ~/%s%s | %s%s" "$user" "$short_dir" "$git_branch" "$model" "$ctx_str"
