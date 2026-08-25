# Architektur

## Ziel

Ankunft ist eine native GTK4-/Libadwaita-Anwendung. Die Benutzeroberfläche,
Synchronisierung und Desktop-Integration werden getrennt gehalten, damit
Netzwerkfehler niemals die Oberfläche blockieren und die Anwendung später auch
im Hintergrund zuverlässig arbeiten kann.

## Module

- `model`: internes, API-unabhängiges Sendungsmodell und Statusdarstellung
- `api`: typisierte Parcel-DTOs, API-Client und lokale Schutzlimits
- `ui`: GNOME-Oberfläche, Filter, Suche und Timeline
- `app`: Lebenszyklus, Aktionen und Desktop-Integration
- `secrets`: Parcel-API-Schlüssel im GNOME-Schlüsselbund
- `storage`: versionierter, atomar geschriebener Offline-Cache mit privaten Dateirechten
- `rate_limit`: persistentes, gleitendes Rate-Limit-Ledger
- `notifications`: datensparsame, gebündelte GNOME-Statusmeldungen

Geplante Module:

- `sync`: Hintergrundabgleich und Änderungsberechnung
- `tray`: optionales StatusNotifierItem ohne Abhängigkeit der Hauptoberfläche

## Parcel-API

- `GET /external/deliveries/?filter_mode=recent`: maximal 20 Aufrufe pro Stunde
- `POST /external/add-delivery/`: maximal 20 Versuche pro Tag; niemals automatisch wiederholen
- `GET /external/supported_carriers.json`: Paketdienst-Metadaten

Die API liefert keine Sendungs-ID. Intern wird deshalb das Paar aus
`carrier_code` und `tracking_number` als stabiler Schlüssel verwendet.

Zeitangaben ohne Epoch-Zeitstempel werden nicht interpretiert, weil ihnen eine
garantierte Zeitzone fehlt. Sie werden so dargestellt, wie Parcel sie liefert.

## Datenschutz

- API-Schlüssel ausschließlich im GNOME-Schlüsselbund
- Trackingnummern, Postleitzahlen, E-Mail-Adressen und API-Antworten niemals in Logs
- lokaler Cache nur mit Benutzerrechten lesbar
- Cache enthält ausschließlich normalisierte Sendungen, niemals API-Schlüssel oder rohe Antworten
- keine Telemetrie und keine zusätzlichen Trackingdienste
- erster erfolgreicher Abgleich erzeugt keine Benachrichtigungsflut

## Nächste Meilensteine

1. Hinzufügen-Dialog mit durchsuchbarer Paketdienstliste
2. optionaler Hintergrundbetrieb und regelmäßiger Abgleich
3. StatusNotifierItem, Autostart und reproduzierbare Paketierung
