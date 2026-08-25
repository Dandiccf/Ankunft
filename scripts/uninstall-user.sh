#!/usr/bin/env bash

set -Eeuo pipefail
IFS=$'\n\t'

readonly APP_ID="io.github.dandiccf.Ankunft"
readonly GETTEXT_DOMAIN="ankunft"

die() {
    printf 'Ankunft konnte nicht deinstalliert werden: %s\n' "$*" >&2
    exit 1
}

warn() {
    printf 'Hinweis: %s\n' "$*" >&2
}

if (( $# != 0 )); then
    die "Dieses Skript akzeptiert keine Argumente."
fi

if (( EUID == 0 )); then
    die "Bitte ohne sudo als der betroffene Desktop-Benutzer ausführen."
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
readonly binary_path="$user_home/.local/bin/ankunft"
readonly applications_directory="$user_data_directory/applications"
readonly desktop_path="$applications_directory/$APP_ID.desktop"
readonly icon_theme_directory="$user_data_directory/icons/hicolor"
readonly icon_path="$icon_theme_directory/scalable/apps/$APP_ID.svg"
readonly dbus_service_path="$user_data_directory/dbus-1/services/$APP_ID.service"
readonly locale_directory="$user_data_directory/locale"

removed_count=0
had_errors=0

remove_exact_file() {
    local target_path="$1"

    if [[ -f "$target_path" || -L "$target_path" ]]; then
        if rm -f -- "$target_path"; then
            ((removed_count += 1))
        else
            warn "Konnte Datei nicht entfernen: $target_path"
            had_errors=1
        fi
    elif [[ -e "$target_path" ]]; then
        warn "Unerwartetes Nicht-Datei-Ziel wurde aus Sicherheitsgründen nicht entfernt: $target_path"
        had_errors=1
    fi
}

remove_exact_file "$desktop_path"
remove_exact_file "$icon_path"
remove_exact_file "$dbus_service_path"
remove_exact_file "$binary_path"

shopt -s nullglob
if [[ -d "$locale_directory" && ! -L "$locale_directory" ]]; then
    for language_directory in "$locale_directory"/*; do
        [[ -d "$language_directory" && ! -L "$language_directory" ]] || continue
        messages_directory="$language_directory/LC_MESSAGES"
        [[ -d "$messages_directory" && ! -L "$messages_directory" ]] || continue

        remove_exact_file "$messages_directory/$GETTEXT_DOMAIN.mo"
        rmdir -- "$messages_directory" 2>/dev/null || true
        rmdir -- "$language_directory" 2>/dev/null || true
    done
elif [[ -L "$locale_directory" ]]; then
    warn "Das Locale-Verzeichnis ist ein Symlink; Übersetzungen wurden aus Sicherheitsgründen nicht entfernt."
    had_errors=1
fi
shopt -u nullglob

if [[ -d "$applications_directory" ]] && command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$applications_directory" \
        || warn "Der Desktop-Anwendungscache konnte nicht aktualisiert werden."
fi

if [[ -d "$icon_theme_directory" ]] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -f -t "$icon_theme_directory" \
        || warn "Der GNOME-Symbolcache konnte nicht aktualisiert werden."
fi

if command -v gdbus >/dev/null 2>&1; then
    gdbus call --session \
        --dest org.freedesktop.DBus \
        --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.ReloadConfig >/dev/null \
        || warn "Der D-Bus-Aktivierungscache konnte nicht sofort aktualisiert werden."
fi

printf 'Ankunft wurde aus der Benutzerinstallation entfernt (%d Dateien).\n' "$removed_count"
printf 'API-Schlüssel, Einstellungen und Offline-Daten wurden bewusst nicht gelöscht.\n'

if (( had_errors != 0 )); then
    exit 1
fi
