# Especificación: Branch Promotion Integrity

> Compañero en español. Canónico / inglés: `spec.md` (IDs de requisitos y escenarios 1:1).

## Purpose

Define la política documentada de promoción de ramas y backport de hotfix, y el contrato
observable de la verificación recurrente que detecta cuándo un hotfix aterrizado en `main` no
llegó a `develop`. Solo detección y reporte — bloqueo, aplicación forzosa o remediación
automatizada quedan fuera de alcance.

## Requirements

### Requirement: The Branching Model And Hotfix Backport Obligation Are Documented

La documentación orientada a contribuidores MUST declarar el modelo de ramas — las ramas de
feature se promueven a `develop` vía pull request, `develop` se promueve a `main` vía pull
request, y un hotfix se ramifica desde `main`, aterriza en `main` vía pull request, y MUST
backportearse posteriormente a `develop` — como una política explícita y descubrible.

#### Scenario: Un contribuidor encuentra el modelo de ramas y la obligación de hotfix

- GIVEN un contribuidor lee la documentación para contribuidores
- WHEN busca la política de promoción de ramas o de hotfix
- THEN encuentra el modelo de ramas y la obligación explícita de backportear el cambio de un
  hotfix a `develop`

### Requirement: Backport Drift Detection Runs On A Recurring Schedule

La verificación de detección de drift MUST correr en un schedule recurrente, independiente de
cualquier push o merge puntual, de modo que un hotfix legítimamente aún no backporteado no
necesite dispararla.

#### Scenario: La detección de drift corre sin un push o merge que la dispare

- GIVEN no acaba de ocurrir ningún push ni merge
- WHEN transcurre el intervalo programado para la detección de drift
- THEN la verificación de detección de drift corre

### Requirement: A Main-Only Change Absent From Develop Is Reported As Drift, After A Grace Period

La verificación de detección de drift MUST reportar cuándo el cambio de un commit existe en
`main` pero no existe un cambio equivalente en `develop`, una vez que ese commit supera un
período de gracia fijo. Dentro del período de gracia, un cambio aún no backporteado MUST NOT
reportarse, de modo que un contribuidor tenga una ventana real para backportear antes de que la
verificación trate la ausencia como drift.

#### Scenario: Un hotfix no backporteado más viejo que el período de gracia se reporta

- GIVEN un commit de hotfix aterrizó en `main`, su cambio está ausente en `develop`, y el commit
  es más viejo que el período de gracia
- WHEN la detección de drift corre la próxima vez
- THEN el cambio de ese hotfix se reporta como drift

#### Scenario: Un hotfix no backporteado dentro del período de gracia todavía no se reporta

- GIVEN un commit de hotfix aterrizó en `main`, su cambio está ausente en `develop`, y el commit
  es más nuevo que el período de gracia
- WHEN la detección de drift corre la próxima vez
- THEN el cambio de ese hotfix no se reporta como drift

### Requirement: Backport Recognition Uses Patch Equivalence, Not Commit Identity

La verificación de detección de drift MUST determinar si un cambio exclusivo de main llegó a
`develop` comparando el contenido efectivo del cambio, no comparando SHAs de commit. Un cambio
backporteado por cherry-pick, que produce una identidad de commit distinta a la del original,
MUST NOT reportarse como drift una vez que su contenido equivalente está presente en `develop`.

#### Scenario: Un backport por cherry-pick no se reporta como drift

- GIVEN un commit de hotfix aterrizó en `main` y luego fue backporteado a `develop` por
  cherry-pick, produciendo una identidad de commit distinta
- WHEN la detección de drift corre la próxima vez
- THEN el cambio de ese hotfix no se reporta como drift

#### Scenario: Un backport mergeado normalmente no se reporta como drift

- GIVEN un commit de hotfix aterrizó en `main` y su cambio llegó a `develop` mediante un merge
  regular
- WHEN la detección de drift corre la próxima vez
- THEN el cambio de ese hotfix no se reporta como drift

### Requirement: Drift Detection Is Non-Blocking, Report-Only, With No Automated Remediation

La verificación de detección de drift MUST NOT bloquear, fallar, ni impedir ningún merge, push u
otra operación del repositorio. MUST NOT crear un pull request, commit, ni ninguna otra acción que
modifique el repositorio en nombre de un contribuidor.

#### Scenario: La detección de drift reporta sin bloquear ninguna operación

- GIVEN existe drift entre `main` y `develop`
- WHEN un contribuidor mergea un pull request no relacionado o hace push a cualquiera de las dos
  ramas
- THEN esa operación tiene éxito y no es bloqueada ni demorada por el reporte de drift

#### Scenario: La detección de drift no toma ninguna acción que modifique el repositorio

- GIVEN se reporta drift
- WHEN se inspecciona el repositorio después
- THEN ningún pull request, commit o rama fue creado ni modificado por la verificación de
  detección de drift

## Non-Goals

- Abrir automáticamente un pull request de backport para un drift detectado.
- Bloquear un merge, push o release a causa de drift detectado.
- Cualquier cambio en las reglas de protección de ramas, revisores requeridos, o checks de estado
  requeridos.
- Detectar drift en cualquier par de ramas que no sea `main` y `develop`.
