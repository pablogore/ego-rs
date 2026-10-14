# Especificación: Release Automation

> Compañero en español. Canónico / inglés: `spec.md` (IDs de requisitos y escenarios 1:1).

## Purpose

Define qué constituye un release de este repositorio: cuándo se corta exactamente uno, cómo se
deriva su versión a partir del historial de commits, y qué deben contener de forma observable el
tag publicado y el GitHub Release. Limitado a promociones a `main`; nada aquí gobierna `develop`,
ramas de feature, ni el versionado a nivel de crate.

## Requirements

### Requirement: A Release Is Cut Only, And Exactly Once, As A Direct Result Of A Push Landing On Main

El sistema MUST crear exactamente un tag de git nuevo y exactamente un GitHub Release nuevo como
resultado directo de un cambio que aterriza en `main`. El sistema MUST NOT crear un tag ni un
Release como resultado de un cambio que aterriza en `develop`, en cualquier otra rama, o en un pull
request abierto o actualizado.

#### Scenario: Un merge a main produce exactamente un tag y un release

- GIVEN un pull request se mergea a `main`
- WHEN el merge aterriza
- THEN existe exactamente un tag de git nuevo apuntando al commit resultante, y exactamente un
  GitHub Release nuevo que referencia ese tag

#### Scenario: Un merge a develop no produce tag ni release

- GIVEN un pull request se mergea a `develop`
- WHEN el merge aterriza
- THEN no existe ningún tag de git nuevo ni ningún GitHub Release nuevo como resultado

#### Scenario: Un pull request abierto o actualizado no produce tag ni release

- GIVEN un pull request se abre o se actualiza contra cualquier rama
- WHEN el CI corre para ese pull request
- THEN no se crea ningún tag de git nuevo ni ningún GitHub Release nuevo

### Requirement: Version Is Derived From Conventional Commits Since The Previous Release, Staying In 0.x

La versión aplicada a un tag de release MUST calcularse a partir de los Conventional Commits
alcanzables en el rango entre el tag de release anterior (exclusivo) y el commit del nuevo release
(inclusivo). La versión calculada MUST seguir el versionado semántico según los tipos de commit de
ese rango y MUST permanecer dentro de la línea mayor `0.x` a menos y hasta que una decisión
explícita cambie esa base. Antes de que exista cualquier tag de release previo, la versión del
repositorio MUST comenzar en `v0.1.0`.

#### Scenario: El primer release siembra la versión base

- GIVEN no existe ningún tag de release previo en `main`
- WHEN se corta el primer release
- THEN el tag resultante es `v0.1.0`

#### Scenario: Un rango de commits determina la siguiente versión

- GIVEN un rango de Conventional Commits desde el tag de release anterior
- WHEN se corta un release
- THEN la versión del nuevo tag refleja el versionado semántico aplicado a los tipos de commit de
  ese rango, y permanece dentro de la línea mayor `0.x`

### Requirement: Release Notes Publish Only To The GitHub Release Body, Never To The Repository Tree

El contenido de changelog generado para un release MUST publicarse como el cuerpo del GitHub
Release de ese release. Ningún commit que agregue, modifique o elimine algún archivo MUST crearse
en el repositorio como parte de cortar un release, y ningún commit MUST enviarse a `main` ni a
`develop` como resultado del proceso de release. Ninguna regla de protección de ramas en cualquier
rama MUST cambiarse, relajarse ni evadirse como resultado del proceso de release.

#### Scenario: El cuerpo del release lleva el contenido del changelog

- GIVEN se corta un release para un rango de Conventional Commits
- WHEN se crea el GitHub Release
- THEN su cuerpo contiene el contenido del changelog para ese rango

#### Scenario: Cortar un release no produce cambios de archivo ni commits

- GIVEN un release se acaba de cortar
- WHEN se inspeccionan el árbol del repositorio y el historial de ramas
- THEN ningún archivo fue agregado, modificado ni eliminado, y ningún commit fue enviado a `main`
  ni a `develop` como parte del proceso de release

### Requirement: The Release Body Is A Human-Readable Record Grouped By Conventional-Commit Type

El cuerpo del release MUST listar los Conventional Commits del rango del release, agrupados por su
tipo de commit (por ejemplo: features, fixes, breaking changes), en una forma legible por una
persona sin consultar git directamente.

#### Scenario: El cuerpo del release agrupa los commits por tipo

- GIVEN un release cuyo rango contiene commits de más de un tipo de Conventional Commit
- WHEN se lee el cuerpo del release
- THEN los commits aparecen agrupados por tipo, y un lector puede identificar qué cambió sin
  ejecutar ningún comando git

### Requirement: Cutting A Release Creates No Push-Triggered Retrigger Loop On Main

Dado que cortar un release no realiza ningún push a ninguna rama, MUST NOT provocar por sí mismo
otra ejecución de ningún proceso disparado por un push a `main`.

#### Scenario: Ningún proceso secundario disparado por push resulta de un release

- GIVEN un release se acaba de cortar como resultado de un push a `main`
- WHEN el proceso de release termina
- THEN no se genera ningún evento de push adicional a `main`, y ninguna ejecución adicional de un
  proceso disparado por push a main resulta del release mismo

### Requirement: Release Versioning Is Independent Of Crate Manifests

La versión aplicada a un tag de release MUST ser independiente de, y MUST NOT modificar, ningún
manifiesto de versión de crate en el repositorio.

#### Scenario: Cortar un release no produce cambios de manifiesto

- GIVEN un release se acaba de cortar
- WHEN se inspecciona cada manifiesto de crate en el workspace
- THEN ninguno de sus campos de versión cambió como resultado

## Non-Goals

- Versionado por crate o publicación en crates.io.
- Un archivo `CHANGELOG.md` versionado en el árbol del repositorio.
- Cambios en el comportamiento de gating o en los triggers de `production-gate.yml`.
- Commits firmados, exigencia de historial lineal, o cualquier cambio en la configuración de
  protección de ramas.
- Un release cortado desde cualquier rama que no sea `main`, o más de un release por promoción.
