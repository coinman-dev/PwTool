#!/usr/bin/env bash
#
# clean-artifacts.sh — убрать из дерева результаты сборки, кэши и мусор ОС,
# чтобы архив для бэкапа нёс только исходники и то, без чего не собрать
# программу.
#
# ХРАНИТСЯ (не трогаем):
#   - весь код, Cargo.toml/Cargo.lock, vendor/ (eframe с патчем), assets/,
#     doc/, packaging/, tools/;
#   - заметки и история: .git, .ai, skills/ — .ai и skills лежат вне
#     репозитория, и архив для них единственная копия;
#   - tmp/ — свои скрипты вне git (sync-wsl.sh, снимки окон и т. п.): идут в
#     архив, --force их не трогает.
#
# УДАЛЯЕТСЯ по --force (пересоберётся само):
#   - target/ (обычно больше десятка гигабайт) — КРОМЕ готовых пакетов
#     (target/dist/, *.deb, *.exe, *.AppImage в корне target/), которые держат
#     для проверки, и target/manuals/ (PDF-руководства: для пересборки нужен
#     LibreOffice);
#   - __pycache__ и мусор ОС и редактора (*Zone.Identifier, .DS_Store, *.swp,
#     *.orig, *.rs.bk, *~).
#
# НЕ УДАЛЯЕТСЯ, но и В АРХИВ НЕ ИДЁТ:
#   - .ref/ — распакованный оригинал Password Tech и префикс wine для эталонов
#     (скачивается заново, см. .ai/PLAN.md);
#   - собранное в tmp/ (*.exe, *.deb, *.tar.gz, *.AppImage, pwtool-linux-*,
#     каталоги target/) — удаляется только по --tmp.
#
# ДОПОЛНИТЕЛЬНО, только по своим ключам (вместе с --force; без него — показ):
#   --agents рабочие копии и ветки агентов: .claude/worktrees/* без
#            незакоммиченных правок и ветки worktree-agent-*, все правки
#            которых уже есть в dev (по содержимому, `git cherry`); перед
#            удалением ветки сохраняются в backup/agent-branches-<время>.bundle.
#            Ветка с правками, которых в dev нет, остаётся — решать человеку.
#   --tmp    собранное в tmp/ (см. выше); скрипты и прочие файлы остаются.
#   --all    = --agents --tmp
#
# Использование:
#   tools/clean-artifacts.sh                  # ПРОБНЫЙ ПРОГОН — что уйдёт и сколько это
#   tools/clean-artifacts.sh --force          # собственно удалить артефакты
#   tools/clean-artifacts.sh --all            # показать и агентов, и собранное в tmp/
#   tools/clean-artifacts.sh --force --all    # удалить артефакты, агентов и собранное в tmp/
#   tools/clean-artifacts.sh --archive [OUT]  # собрать лёгкий .tar.gz (ничего не удаляя)
#                                             # по умолчанию — в ТЕКУЩИЙ каталог
#   tools/clean-artifacts.sh --archive --with-git [OUT]   # ...вместе с историей .git
#   tools/clean-artifacts.sh --help
#
set -euo pipefail

INVOKE_DIR="$(pwd)"          # откуда запустили — туда и ляжет архив
# Абсолютный путь к самому скрипту — ЗАПОМНИТЬ ДО перехода в корень: `--help`
# читает справку из собственного текста, а после `cd` относительный $0 уже
# никуда не ведёт.
SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE="$(basename "$ROOT")"
cd "$ROOT"

# Готовые пакеты и руководства в target/: остаются (имена от target/).
TARGET_KEEP_DIRS=("dist" "manuals")
TARGET_KEEP_GLOBS=("*.deb" "*.exe" "*.AppImage")

# Мусор ОС и редактора (ищется по всему дереву).
JUNK_GLOBS=("*.DS_Store" "Thumbs.db" "*Zone.Identifier" "*.swp" "*.swo" "*.orig" "*.rej" "*~" "*.rs.bk" "*.pyc")

# Собранное в tmp/: в архив не идёт, удаляется только по --tmp.
TMP_BUILT_GLOBS=("*.exe" "*.deb" "*.tar.gz" "*.AppImage" "pwtool-linux-*")

human() { du -sh "$1" 2>/dev/null | cut -f1; }

usage() { sed -n '2,/^set -euo/p' "$SELF" | grep '^#' | sed 's/^# \{0,1\}//'; exit 0; }

# find по дереву без .git/target/vendor/tmp/.ref — иначе обход 15 ГБ target
# ради пары файлов мусора; в vendor/ лежит чужой код как есть (Cargo.toml.orig
# там — часть крейта), tmp/ чистится только по --tmp, .ref/ — префикс wine с
# файлами Windows, их «мусор» трогать нельзя. Каталог, куда find не пустили, —
# не повод обрывать весь прогон.
prune_find() {
  find . -path ./.git -prune -o -path ./target -prune -o -path ./vendor -prune \
       -o -path ./tmp -prune -o -path ./.ref -prune -o "$@" || true
}

# Входит ли имя (без каталога) в один из шаблонов.
matches_any() {
  local name="$1"; shift
  local g
  for g in "$@"; do
    # shellcheck disable=SC2053
    [[ "$name" == $g ]] && return 0
  done
  return 1
}

# ── разбор аргументов ──────────────────────────────────────────────────────
MODE="dry"          # dry | force | archive
WITH_GIT=0
DO_AGENTS=0
DO_TMP=0
OUT=""
while [ $# -gt 0 ]; do
  case "$1" in
    --force|-f)   MODE="force" ;;
    --archive|-a) MODE="archive" ;;
    --with-git)   WITH_GIT=1 ;;
    --agents)     DO_AGENTS=1 ;;
    --tmp)        DO_TMP=1 ;;
    --all)        DO_AGENTS=1; DO_TMP=1 ;;
    -h|--help)    usage ;;
    -*)           echo "неизвестный ключ: $1" >&2; exit 2 ;;
    *)            OUT="$1" ;;   # позиционный → путь архива
  esac
  shift
done

# ── режим архива ───────────────────────────────────────────────────────────
if [ "$MODE" = "archive" ]; then
  ts="$(date +%Y%m%d-%H%M%S)"
  if [ -z "$OUT" ]; then
    OUT="$INVOKE_DIR/${BASE}-backup-${ts}.tar.gz"
  elif [ "${OUT#/}" = "$OUT" ]; then
    OUT="$INVOKE_DIR/$OUT"                                    # относительный — от каталога запуска
  fi

  excludes=()
  excludes+=(--exclude='*.tar.gz')      # не паковать архив в самого себя и не тащить старые
  excludes+=(--exclude="$BASE/target" --exclude="$BASE/.ref")
  excludes+=(--exclude='__pycache__')
  for g in "${JUNK_GLOBS[@]}"; do excludes+=(--exclude="$g"); done
  # собранное в tmp/; скрипты оттуда идут в архив
  excludes+=(--exclude="$BASE/tmp/*/target" --exclude="$BASE/tmp/target")
  for g in "${TMP_BUILT_GLOBS[@]}"; do excludes+=(--exclude="$BASE/tmp/$g" --exclude="$BASE/tmp/*/$g"); done
  [ "$WITH_GIT" -eq 0 ] && excludes+=(--exclude="$BASE/.git")

  echo "Собираю архив (весь код плюс .ai, skills/ и скрипты из tmp/ —"
  echo "того, что вне git, это единственная копия):"
  echo "  -> $OUT"
  [ "$WITH_GIT" -eq 0 ] && echo "  (история .git не входит — нужна, добавьте --with-git)"
  [ -d .ref ] && echo "  (.ref/ не входит: оригинал Password Tech и префикс wine скачиваются заново)"
  tar -C "$(dirname "$ROOT")" -czf "$OUT" "${excludes[@]}" "$BASE"
  echo "Готово. Размер архива: $(human "$OUT")"
  exit 0
fi

# ── чистка / пробный прогон ────────────────────────────────────────────────
total_kb=0
found=0
note() { printf '  %-9s %s\n' "$1" "$2"; }

echo "Артефакты под $ROOT:"
if [ -d target ] && [ ! -L target ]; then
  kept_kb=0
  kept_names=()
  gone_kb=0
  gone=()
  for e in target/* target/.[!.]*; do
    [ -e "$e" ] || continue
    name="${e#target/}"
    if { [ -d "$e" ] && matches_any "$name" "${TARGET_KEEP_DIRS[@]}"; } ||
       { [ -f "$e" ] && matches_any "$name" "${TARGET_KEEP_GLOBS[@]}"; }; then
      kept_kb=$(( kept_kb + $(du -sk "$e" | cut -f1) ))
      kept_names+=("$name")
    else
      gone_kb=$(( gone_kb + $(du -sk "$e" | cut -f1) ))
      gone+=("$e")
    fi
  done
  if [ ${#gone[@]} -gt 0 ]; then
    found=1
    note "$(( gone_kb / 1024 ))M" "target/ (кроме пакетов и руководств)"
    total_kb=$(( total_kb + gone_kb ))
    [ "$MODE" = "force" ] && rm -rf -- "${gone[@]}"
  fi
  if [ ${#kept_names[@]} -gt 0 ]; then
    note "" "остаются в target/ ($(( kept_kb / 1024 ))M): ${kept_names[*]}"
  fi
fi

# __pycache__ (каталоги целиком).
pyc_dirs=$(prune_find -type d -name __pycache__ -print -prune 2>/dev/null)
if [ -n "$pyc_dirs" ]; then
  found=1
  while IFS= read -r d; do
    note "$(human "$d")" "${d#./}/"
    total_kb=$(( total_kb + $(du -sk "$d" | cut -f1) ))
    [ "$MODE" = "force" ] && rm -rf -- "$d"
  done <<< "$pyc_dirs"
fi

# Мусор ОС и редактора: считаем, по --force удаляем.
junk_args=()
for g in "${JUNK_GLOBS[@]}"; do junk_args+=(-name "$g" -o); done
unset 'junk_args[${#junk_args[@]}-1]'   # убрать хвостовой -o
junk_count=$(prune_find -type f \( "${junk_args[@]}" \) -print 2>/dev/null | wc -l | tr -d ' ')
if [ "$junk_count" -gt 0 ]; then
  found=1
  note "" "$junk_count файл(ов) мусора ОС/редактора (Zone.Identifier, .DS_Store, swap …)"
  [ "$MODE" = "force" ] && prune_find -type f \( "${junk_args[@]}" \) -print0 2>/dev/null | xargs -0 -r rm -f
fi

if [ "$found" -eq 0 ]; then
  echo "  (чисто — удалять нечего)"
else
  echo
  if [ "$total_kb" -ge 1024 ]; then
    printf 'Освободится: ~%s МБ\n' "$(( total_kb / 1024 ))"
  else
    printf 'Освободится: ~%s КБ\n' "$total_kb"
  fi
fi

if [ -d .ref ]; then
  echo
  echo "  .ref/ ($(human .ref)) не трогается и в архив не идёт: оригинал Password Tech и префикс wine."
fi

# ── рабочие копии и ветки агентов ──────────────────────────────────────────
if [ "$DO_AGENTS" -eq 1 ]; then
  echo
  echo "Агенты (рабочие копии .claude/worktrees и ветки worktree-agent-*):"
  if [ -d .claude/worktrees ]; then
    for wt in .claude/worktrees/*/; do
      [ -d "$wt" ] || continue
      wt="${wt%/}"
      if [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]; then
        note "$(human "$wt")" "$wt — есть незакоммиченные правки, остаётся"
        continue
      fi
      note "$(human "$wt")" "$wt (рабочая копия)"
      [ "$MODE" = "force" ] && git worktree remove --force "$wt"
    done
  fi
  [ "$MODE" = "force" ] && git worktree prune
  gone=()
  for b in $(git branch --format='%(refname:short)' | grep '^worktree-agent-' || true); do
    if git worktree list --porcelain | grep -qx "branch refs/heads/$b"; then
      note "" "$b — занята рабочей копией, остаётся"
    elif git cherry dev "$b" | grep -q '^+'; then
      note "" "$b — есть правки не из dev ($(git log -1 --format=%cs "$b")), остаётся: $(git log -1 --format=%s "$b" | cut -c1-60)"
    else
      note "" "$b (всё уже в dev)"
      gone+=("$b")
    fi
  done
  [ ${#gone[@]} -eq 0 ] && [ ! -d .claude/worktrees ] && echo "  (нет)"
  if [ "$MODE" = "force" ] && [ ${#gone[@]} -gt 0 ]; then
    mkdir -p backup
    bundle="backup/agent-branches-$(date +%Y%m%d-%H%M%S).bundle"
    git bundle create "$bundle" "${gone[@]}" >/dev/null 2>&1 && echo "  ветки сохранены: $bundle"
    git branch -D "${gone[@]}" >/dev/null
  fi
fi

# ── собранное в tmp/ ───────────────────────────────────────────────────────
if [ "$DO_TMP" -eq 1 ]; then
  echo
  echo "Собранное в tmp/ (скрипты остаются):"
  tmp_kb=0
  tmp_found=0
  if [ -d tmp ] && [ ! -L tmp ]; then
    # каталоги target/ (пробные проекты) — целиком
    while IFS= read -r d; do
      [ -n "$d" ] || continue
      tmp_found=1
      note "$(human "$d")" "$d/"
      tmp_kb=$(( tmp_kb + $(du -sk "$d" | cut -f1) ))
      [ "$MODE" = "force" ] && rm -rf -- "$d"
    done < <(find tmp -maxdepth 2 -type d -name target -print -prune 2>/dev/null || true)
    built_args=()
    for g in "${TMP_BUILT_GLOBS[@]}"; do built_args+=(-name "$g" -o); done
    unset 'built_args[${#built_args[@]}-1]'
    while IFS= read -r f; do
      [ -n "$f" ] || continue
      tmp_found=1
      note "$(human "$f")" "$f"
      tmp_kb=$(( tmp_kb + $(du -sk "$f" | cut -f1) ))
      [ "$MODE" = "force" ] && rm -f -- "$f"
    done < <(find tmp -maxdepth 2 -type f \( "${built_args[@]}" \) -print 2>/dev/null || true)
  fi
  if [ "$tmp_found" -eq 0 ]; then
    echo "  (нет)"
  else
    printf '  итого: ~%s МБ\n' "$(( tmp_kb / 1024 ))"
    found=1
  fi
fi

if [ "$MODE" = "force" ]; then
  echo
  echo "Удалено. ✓  (код, .ai, skills/, скрипты в tmp/ и готовые пакеты на месте)"
  echo "  вернуть сборку: cargo build"
elif [ "$found" -ne 0 ]; then
  echo
  echo "Это был ПРОБНЫЙ ПРОГОН — ничего не удалено."
  echo "  удалить:            tools/clean-artifacts.sh --force [--agents] [--tmp | --all]"
  echo "  вместо этого архив: tools/clean-artifacts.sh --archive"
fi
