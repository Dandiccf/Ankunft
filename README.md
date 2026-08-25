# Ankunft

Ankunft ist ein moderner, nativer GNOME-Client für Parcel Premium. Das Projekt
befindet sich in einer frühen Prototypphase und verwendet aktuell ausschließlich
lokale Beispieldaten.

## Aktueller Stand

- native GTK4-/Libadwaita-Oberfläche
- adaptive Drei-Spalten-Ansicht für Filter, Sendungen und Details
- Suche und Statusfilter
- Ereignis-Timeline und Zustellinformationen
- vorbereitete, noch nicht mit einem Konto verbundene Parcel-API-Schicht
- keine Zugangsdaten im Quellcode oder in Konfigurationsdateien

## Starten

```bash
cargo run
```

Benötigt werden Rust, GTK 4 und Libadwaita einschließlich der jeweiligen
Entwicklerpakete.

## Sicherheit

Ein später verwendeter Parcel-API-Schlüssel wird ausschließlich über den
GNOME-Schlüsselbund gespeichert. Bitte niemals einen API-Schlüssel in Issues,
Logs oder den Quellcode einfügen.

