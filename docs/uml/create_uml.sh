!#/usr/bin/env bash

SRC_DIR=$1

OUT_DIR=$2

mkdir -p "$OUT_DIR"

generate_uml() {
  local file=$1
  local filename=$(basename "$file" | sed 's/\.[^.]*$//') # имя файла без расширения
  local output_file="$OUT_DIR/${filename}.png"

  plantuml -tpng -o "$OUT_DIR" "$file"

  echo "UML generated: $output_file"
}

inotifywait -m -e close_write,moved_to,create --format '%w%f' "$SRC_DIR" | while read file; do
  if [[ "$file" == *.puml || "$file" == *.uml ]]; then
    generate_uml "$file"
  fi
done
