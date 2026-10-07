#!/usr/bin/env bash

set -Eeuo pipefail
IFS=$'\n\t'
umask 022

readonly APP_ID="io.github.dandiccf.Ankunft"
readonly GETTEXT_DOMAIN="ankunft"
SCRIPT_DIRECTORY="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
PROJECT_DIRECTORY="$(cd -- "$SCRIPT_DIRECTORY/.." && pwd -P)"
readonly SCRIPT_DIRECTORY PROJECT_DIRECTORY

die() {
    printf 'Ankunft konnte nicht installiert werden: %s\n' "$*" >&2
    exit 1
}

warn() {
    printf 'Hinweis: %s\n' "$*" >&2
}

if (( $# != 0 )); then
    die "Dieses Skript akzeptiert keine Argumente."
fi

if (( EUID == 0 )); then
    die "Bitte ohne sudo als der gewünschte Desktop-Benutzer ausführen."
fi

user_home="${HOME:-}"
[[ -n "$user_home" && "$user_home" == /* && "$user_home" != "/" ]] \
    || die "HOME ist nicht auf ein sicheres absolutes Benutzerverzeichnis gesetzt."

if [[ -n "${XDG_DATA_HOME:-}" && "${XDG_DATA_HOME}" == /* ]]; then
    user_data_directory="$XDG_DATA_HOME"
else
    if [[ -n "${XDG_DATA_HOME:-}" ]]; then
        warn "Das relative XDG_DATA_HOME wird gemäß XDG-Spezifikation ignoriert."
    fi
    user_data_directory="$user_home/.local/share"
fi
[[ "$user_data_directory" != "/" ]] || die "XDG_DATA_HOME darf nicht das Wurzelverzeichnis sein."

readonly user_home user_data_directory
readonly binary_source="$PROJECT_DIRECTORY/target/release/ankunft"
readonly desktop_source="$PROJECT_DIRECTORY/data/$APP_ID.desktop"
readonly service_source="$PROJECT_DIRECTORY/data/$APP_ID.service.in"
readonly icon_source="$PROJECT_DIRECTORY/icons/$APP_ID.svg"
readonly metainfo_source="$PROJECT_DIRECTORY/data/$APP_ID.metainfo.xml"

readonly binary_directory="$user_home/.local/bin"
readonly applications_directory="$user_data_directory/applications"
readonly icon_theme_directory="$user_data_directory/icons/hicolor"
readonly icon_directory="$icon_theme_directory/scalable/apps"
readonly dbus_service_directory="$user_data_directory/dbus-1/services"
readonly locale_directory="$user_data_directory/locale"

readonly binary_destination="$binary_directory/ankunft"
readonly desktop_destination="$applications_directory/$APP_ID.desktop"
readonly service_destination="$dbus_service_directory/$APP_ID.service"
readonly icon_destination="$icon_directory/$APP_ID.svg"
readonly metainfo_destination="$user_data_directory/metainfo/$APP_ID.metainfo.xml"

require_regular_file() {
    local source_path="$1"
    local description="$2"
    [[ -f "$source_path" && -r "$source_path" ]] \
        || die "$description fehlt oder ist nicht lesbar: $source_path"
}

require_regular_file "$binary_source" "Das Release-Binary"
[[ -x "$binary_source" ]] \
    || die "Das Release-Binary ist nicht ausführbar. Bitte zuerst 'cargo build --release' ausführen."
require_regular_file "$desktop_source" "Die Desktop-Datei"
require_regular_file "$service_source" "Die D-Bus-Service-Datei"
require_regular_file "$icon_source" "Das Anwendungssymbol"
require_regular_file "$metainfo_source" "Die AppStream-Metadaten"

if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$desktop_source" \
        || die "Die Desktop-Datei hat die Validierung nicht bestanden."
fi

grep -Fqx "Name=$APP_ID" "$service_source" \
    || die "Die D-Bus-Service-Datei enthält nicht den erwarteten Anwendungsnamen."
grep -Fqx 'Exec=@ANKUNFT_BINARY@ --gapplication-service' "$service_source" \
    || die "Die D-Bus-Service-Vorlage enthält nicht den erwarteten Exec-Platzhalter."

temporary_paths=()
cleanup_temporary_files() {
    local temporary_path
    for temporary_path in "${temporary_paths[@]}"; do
        if [[ -f "$temporary_path" || -L "$temporary_path" ]]; then
            rm -f -- "$temporary_path"
        fi
    done
}
trap cleanup_temporary_files EXIT

install_atomically() {
    local source_path="$1"
    local destination_path="$2"
    local mode="$3"
    local destination_directory destination_name temporary_path

    destination_directory="$(dirname -- "$destination_path")"
    destination_name="$(basename -- "$destination_path")"
    mkdir -p -- "$destination_directory"

    if [[ -e "$destination_path" && ! -f "$destination_path" && ! -L "$destination_path" ]]; then
        die "Das Ziel ist keine reguläre Datei: $destination_path"
    fi

    temporary_path="$(mktemp --tmpdir="$destination_directory" ".${destination_name}.install.XXXXXX")"
    temporary_paths+=("$temporary_path")
    install -m "$mode" -- "$source_path" "$temporary_path"
    sync -f "$temporary_path" 2>/dev/null || true
    mv -fT -- "$temporary_path" "$destination_path"
    sync -f "$destination_directory" 2>/dev/null || true
}

quote_dbus_exec_argument() {
    local value="$1"

    [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] \
        || die "Der Installationspfad enthält einen für D-Bus ungültigen Zeilenumbruch."
    [[ "$value" != *"'"* && "$value" != *\\* ]] \
        || die "Der Installationspfad enthält ein von D-Bus nicht sicher darstellbares Apostroph oder einen Rückstrich."

    printf "'%s'" "$value"
}

render_dbus_service() {
    local source_path="$1"
    local destination_path="$2"
    local line quoted_binary replacement_count=0

    quoted_binary="$(quote_dbus_exec_argument "$binary_destination")"
    while IFS= read -r line || [[ -n "$line" ]]; do
        if [[ "$line" == 'Exec=@ANKUNFT_BINARY@ --gapplication-service' ]]; then
            printf 'Exec=%s --gapplication-service\n' "$quoted_binary"
            ((replacement_count += 1))
        else
            printf '%s\n' "$line"
        fi
    done < "$source_path" > "$destination_path"

    (( replacement_count == 1 )) \
        || die "Der D-Bus-Service konnte nicht eindeutig erzeugt werden."
}

# Validate the final activation path before installing any files so an
# unsupported home-directory name cannot leave a partial installation behind.
quote_dbus_exec_argument "$binary_destination" >/dev/null

# Compile the current source catalogs. Selecting the first Cargo OUT_DIR can
# accidentally install translations from an older build after feature changes.
command -v msgfmt >/dev/null || die "GNU gettext (msgfmt) fehlt."
require_regular_file "$PROJECT_DIRECTORY/po/LINGUAS" "Die Sprachliste"
translation_sources=()
translation_locales=()
declare -A seen_locales=()
while IFS= read -r locale_line || [[ -n "$locale_line" ]]; do
    locale_line="${locale_line%%#*}"
    IFS=$' \t' read -r -a locales_for_line <<< "$locale_line"
    for locale_name in "${locales_for_line[@]}"; do
        [[ "$locale_name" =~ ^[[:alpha:]]{2,3}([_.@-][[:alnum:]]+)*$ ]] \
            || die "Ungültige Sprache: $locale_name"
        [[ -z "${seen_locales[$locale_name]+present}" ]] || die "Doppelte Sprache: $locale_name"
        seen_locales[$locale_name]=1
        catalog_source="$PROJECT_DIRECTORY/po/$locale_name.po"
        require_regular_file "$catalog_source" "Die Übersetzung"
        compiled_catalog="$(mktemp --suffix=.mo)"
        temporary_paths+=("$compiled_catalog")
        msgfmt --check "$catalog_source" -o "$compiled_catalog" || die "Ungültige Übersetzung: $locale_name"
        translation_sources+=("$compiled_catalog")
        translation_locales+=("$locale_name")
    done
done < "$PROJECT_DIRECTORY/po/LINGUAS"

install_atomically "$binary_source" "$binary_destination" 0755
install_atomically "$desktop_source" "$desktop_destination" 0644
install_atomically "$icon_source" "$icon_destination" 0644
install_atomically "$metainfo_source" "$metainfo_destination" 0644

mkdir -p -- "$dbus_service_directory"
rendered_service="$(mktemp --tmpdir="$dbus_service_directory" ".${APP_ID}.service.render.XXXXXX")"
temporary_paths+=("$rendered_service")
render_dbus_service "$service_source" "$rendered_service"
install_atomically "$rendered_service" "$service_destination" 0644

for index in "${!translation_sources[@]}"; do
    locale_name="${translation_locales[$index]}"
    install_atomically \
        "${translation_sources[$index]}" \
        "$locale_directory/$locale_name/LC_MESSAGES/$GETTEXT_DOMAIN.mo" \
        0644
done

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$applications_directory" \
        || warn "Der Desktop-Anwendungscache konnte nicht aktualisiert werden."
else
    warn "update-desktop-database ist nicht verfügbar; GNOME aktualisiert den Eintrag später selbst."
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -f -t "$icon_theme_directory" \
        || warn "Der GNOME-Symbolcache konnte nicht aktualisiert werden."
else
    warn "gtk-update-icon-cache ist nicht verfügbar; GNOME aktualisiert das Symbol später selbst."
fi

if command -v gdbus >/dev/null 2>&1; then
    gdbus call --session \
        --dest org.freedesktop.DBus \
        --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.ReloadConfig >/dev/null \
        || warn "Der D-Bus-Aktivierungscache konnte nicht sofort aktualisiert werden."
fi

printf 'Ankunft wurde für den aktuellen Benutzer installiert.\n'
printf 'Programm: %s\n' "$binary_destination"
printf 'Desktop-Eintrag: %s\n' "$desktop_destination"
printf 'Installierte Übersetzungen: %d\n' "${#translation_sources[@]}"
