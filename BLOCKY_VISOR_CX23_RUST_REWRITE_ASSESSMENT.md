# Blocky Visor: Rust-Rewrite-Bewertung für CX23 (Debian 13)

## Ausgangslage

- Server: Hetzner **CX23**
- OS: **Debian 13**
- Dienste: **Blocky (Docker)**, **SearXNG**, **Caddy**
- Fragestellung: Lohnt sich ein Rewrite des Blocky-Visor-Backends in Rust zur Reduktion von RAM/CPU auf Low-End-VPS?

## Kurzfazit

Ein **vollständiger Rust-Rewrite lohnt sich aktuell eher nicht**.  
Der größere Hebel liegt zuerst in der **Optimierung des bestehenden Go-Sidecars** und im Betriebs-Setup.

## Begründung

1. Blocky Visor ist primär eine statische SPA; das Backend ist ein optionaler, relativ schlanker Go-Sidecar.
2. Die größten Ressourcen-Treiber sind voraussichtlich Datenpfad-Themen (z. B. Log-Verarbeitung, Aggregation, Paging) und nicht primär die Programmiersprache.
3. Ein Full-Rewrite erhöht Komplexität, Migrationsrisiko und Wartungsaufwand.

## Priorisierte Maßnahmen vor einem Rewrite

1. **Streaming/inkrementelles Paging** statt Voll-Laden großer Logs in RAM.
2. **Pre-Aggregation/Indexing** (z. B. SQLite-basiert) für Analytics statt wiederholtem Voll-Parsing.
3. **Caching ausbauen** und Reverse-DNS-Auflösung begrenzen/debouncen.
4. Polling-Intervalle und Logging im Betrieb auf Ressourcenbudget abstimmen.

## Entscheidungs-Matrix

### Go beibehalten

Wenn nach Optimierung:
- RAM stabil im Ziel bleibt,
- CPU unter realistischer Last nicht dauerhaft sättigt,
- p95/p99-Latenz im akzeptablen Bereich bleibt.

### Teil-Rewrite in Rust (nur Hot Paths)

Wenn Profiling zeigt:
- anhaltende Engpässe in Parser/Aggregation,
- weiterhin kritische RAM- oder CPU-Spitzen trotz Go-Optimierung.

### Full-Rewrite in Rust

Nur wenn:
- harte Ressourcen-Ziele sonst nicht erreichbar sind,
- langfristige Wartungskapazität mit Rust gesichert ist.

## Empfehlung für deinen konkreten Stack (CX23 + Blocky + SearXNG + Caddy)

1. Zuerst gezielte Sidecar-Optimierungen + sauberes Betriebs-Tuning.
2. Danach unter realistischen Lastprofilen erneut messen.
3. Nur bei verbleibenden Engpässen: selektiver Rust-Teilrewrite.
4. Full-Rewrite erst als letzte Option.

## Abnahmekriterien (Go/No-Go)

- Kein dauerhafter Swap-Druck unter realistischer Dauerlast.
- Kein kontinuierlicher RAM-Anstieg über längere Laufzeiten.
- Stabile Antwortzeiten bei parallelem DNS-, UI- und Suchverkehr.
- Wenn erfüllt: **kein Full-Rewrite notwendig**.

