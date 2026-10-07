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

Die von Parcel gelieferte Ansicht `recent` wird lokal um bereits beobachtete,
zugestellte Sendungen ergänzt. Dadurch bleibt eine private Historie erhalten,
auch wenn ältere Zustellungen nicht mehr über die eingeschränkte externe API
geliefert werden. Ein vollständiger rückwirkender Import ist über diese API
nicht möglich.

Geplante Module:

- `sync`: Hintergrundabgleich und Änderungsberechnung
- `tray`: optionales StatusNotifierItem ohne Abhängigkeit der Hauptoberfläche

## Parcel-API

- `GET /external/deliveries/?filter_mode=recent`: maximal 20 Aufrufe pro Stunde
- `POST /external/add-delivery/`: maximal 20 Versuche pro Tag; niemals automatisch wiederholen
- `GET /external/supported_carriers.json`: Paketdienst-Metadaten
- Paketdienstsuche und Trackingnummer-Vorschläge: vollständig lokal; die
  öffentliche Parcel-API stellt keine Erkennungsregeln oder Vorschau bereit

Die API liefert keine Sendungs-ID. Intern wird deshalb das Paar aus
`carrier_code` und `tracking_number` als stabiler Schlüssel verwendet.

Eine manuelle „zugestellt“-Markierung bleibt ausschließlich im privaten
Offline-Snapshot. Der originale Parcel-Status bleibt getrennt erhalten und ist
weiterhin die einzige Quelle für Benachrichtigungen. Meldet Parcel später selbst
„zugestellt“, wird die lokale Überschreibung beim atomaren Snapshot-Update
entfernt. Cache-Schema 2 verhindert, dass ältere Builds diese Markierungen beim
Zurückschreiben unbemerkt verlieren.

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

1. Verwaltung und gezieltes Entfernen lokal archivierter Zustellungen
2. optionaler Hintergrundbetrieb und regelmäßiger Abgleich
3. StatusNotifierItem, Autostart und reproduzierbare Paketierung

## Release 0.2.0

Der aktive Prozess prüft verbundene Konten alle 15 Minuten; laufende Requests und
Kontodialoge verhindern überlappende automatische Abrufe. Ein optionaler,
sitzungslokaler Hintergrundmodus hält das versteckte Fenster und den Prozess
aktiv. In Flatpak wird vorher die Erlaubnis des Background-Portals eingeholt.
Beenden beendet auch die Überwachung. Login-Autostart und Tray bleiben optional
geplante Funktionen.

`--demo` erstellt eine getrennte Instanz ohne Zugriff auf Schlüsselbund, Cache,
API-Zähler oder Netzwerk. Die Screenshot- und Startup-Tests verwenden diesen
Modus. Flatpak vendort die Abhängigkeiten aus Cargo.lock und baut ohne Netzwerk
im GNOME-SDK; die Release-Pipeline prüft beide Architekturen vor Veröffentlichung.
