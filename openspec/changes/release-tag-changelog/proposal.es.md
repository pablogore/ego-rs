# Propuesta: release-tag-changelog — Corte de release en `main` + integridad del backport de hotfix

> Compañero en español. Canónico / inglés: `proposal.md` (encabezados 1:1).
> ATOMICITY: PASS (dos slices revertibles de forma independiente). BILINGUAL_SYNC: PASS.

## Intent

El repositorio nunca cortó un release: cero tags, cero GitHub Releases, sin `CHANGELOG.md`, sin
workflow de release. Hacer merge de `develop` → `main` no produce ningún registro durable y legible
por humanos de lo que se publicó, aunque el historial de commits ya es casi 100% Conventional Commits
— la entrada que un changelog necesita ya está presente y sin usar.

Por separado, `develop` está adelantado respecto de `main` por diseño, así que un hotfix que aterriza
en `main` puede no llegar nunca a `develop` de forma silenciosa. Hoy nada lo hace cumplir ni lo
**detecta**. `CONTRIBUTING.md` documenta el CI en detalle pero no dice nada sobre el modelo de ramas
ni sobre la política de hotfix.

## Scope

### In Scope

- Un release se corta **solo** cuando un merge aterriza en `main` — nunca en `develop`, nunca por PR.
  Un release por promoción: un tag anotado de git más un GitHub Release cuyo cuerpo es el changelog de
  conventional commits del rango desde el tag anterior.
- La versión se calcula a partir de los conventional commits de ese rango. Semver a nivel de
  repositorio, sembrado en `v0.1.0` sobre el head actual de `main`, permaneciendo en `0.x` hasta una
  decisión explícita de pasar a 1.0.
- Las notas de release se publican únicamente en el cuerpo del GitHub Release. No se escribe nada en
  el árbol del repositorio, así que ningún commit automatizado apunta jamás a una rama protegida y no
  se introduce ningún token de bypass.
- `CONTRIBUTING.md` incorpora el modelo de ramas (feature → `develop` → `main`; hotfix desde `main` →
  `main` → backport a `develop`) y el proceso de release, igual que ya documenta el CI.
- **Detección de drift de backport**: una verificación recurrente y no bloqueante que reporta cuándo
  `main` contiene un commit cuyo cambio está ausente en `develop`. Un backport hecho por cherry-pick
  NO DEBE reportarse como drift.

### Out of Scope

- Versionado por crate y publicación en crates.io. Los 22 miembros conservan su `0.1.0` independiente;
  la versión del tag del repositorio es independiente de las versiones de los crates y no se toca
  ningún manifiesto.
- `CHANGELOG.md` en el árbol del repositorio. Las notas viven en Releases y son regenerables desde el
  historial de git bajo demanda; un archivo versionado solo aporta una copia en el árbol al costo de
  un PR de bot por cada release.
- Bloquear o remediar automáticamente un backport omitido (PRs de backport abiertos automáticamente,
  bloqueo de merge). Solo detección y reporte.
- Cambiar la protección de ramas, agregar una app/PAT de bypass, o exigir commits firmados / historial
  lineal.
- Cambios en el comportamiento de gating de `production-gate.yml`, y cualquier promoción de
  `.shipwright/workflow.yaml`.

## Capabilities

### New Capabilities

- `release-automation`: qué constituye un release, cuándo se corta exactamente uno, cómo se deriva su
  versión de los conventional commits, y qué deben contener el tag y el cuerpo del Release publicados.
- `branch-promotion-integrity`: la política documentada de promoción/hotfix, y el contrato observable
  de la detección de drift — incluyendo que un backport por cherry-pick no es drift.

### Modified Capabilities

- Ninguna. Ninguna spec existente en `openspec/specs/` cubre CI, release ni ramas.

## Approach

Usar **git-cliff** en su forma sin commit, accionado por un workflow disparado en push a `main`.
git-cliff calcula la siguiente versión y renderiza las notas a partir de los mismos conventional
commits ya presentes en el historial; el tag y el Release se crean a través de la API de GitHub con el
`GITHUB_TOKEN` por defecto.

Se elige por sobre release-please porque *elimina* el problema de protección de ramas en lugar de
negociar con él: nunca se hace push a una rama protegida, así que no hay un PR de bot permanente que
aprobar en cada release, no hay estancamiento por un check requerido que nunca se dispara, y no hay
guarda de bucle infinito que mantener. También coincide con el precedente existente del repositorio de
fijar un binario CLI externo (`shipwright-validation.yml`) en vez de adoptar una Action del
marketplace. semantic-release queda descalificado — la investigación confirmó que su modelo de push
directo no puede pasar esta protección. cargo-release resuelve la publicación por crate, que
explícitamente no es el problema.

La detección de drift compara ambas ramas por equivalencia de parche en vez de por identidad de
commit, así que un hotfix aplicado con cherry-pick se lee como backporteado. Corre por schedule y no
inmediatamente después del merge del hotfix, porque `develop` está legítimamente atrasado durante la
ventana de backport.

La entrega son dos slices: (1) workflow de release + tag base, (2) documentación de ramas/hotfix +
verificación de drift.

## Affected Areas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `.github/workflows/` | Nuevo | Workflow de release en push a `main`; verificación de drift por schedule |
| `CONTRIBUTING.md` | Modificado | Modelo de ramas, política de hotfix/backport, proceso de release |
| Tags de git / GitHub Releases | Nuevo | `v0.1.0` base, luego uno por promoción a `main` |
| `.github/workflows/production-gate.yml` | Sin cambios | Sigue corriendo en push a `main`; no se hace push de vuelta, así que no hay bucle |
| `Cargo.toml` raíz, los 22 manifiestos miembro | Sin cambios | Sin bump de versión; el tag del repositorio es independiente |
| `openspec/config.yaml` | Sin cambios | Este cambio no necesita una regla de fase release |

## Risks

| Riesgo | Probabilidad | Mitigación |
|--------|--------------|------------|
| Greenfield: no hay tag previo contra el cual validar la salida | Alta | Sembrar el tag base manualmente y verificar el primer corte a mano antes de confiar en el workflow |
| El cálculo de versión se comporta mal bajo semántica `0.x` | Media | Fijar la versión de la herramienta; verificar la versión calculada en el primer corte antes de publicar |
| Falsos positivos de la verificación de drift con backports por cherry-pick, y se termina ignorando | Media | El contrato de equivalencia de parche es un requisito de spec, no un detalle de implementación |
| Las notas de release son el único changelog; algún consumidor quiere un archivo en el árbol | Baja | Las notas son regenerables desde el historial en cualquier momento hacia un PR revisado normal |
| El tag aterriza sobre un commit sin firmar y con historial no lineal | Baja | Aceptado explícitamente; el endurecimiento de la cadena de suministro es un cambio aparte |

## Rollback Plan

Ambos slices son archivos aditivos. Borrar el archivo del workflow detiene toda la automatización de
inmediato. Los artefactos publicados se pueden quitar sin tocar el código fuente: borrar el GitHub
Release, y luego borrar el tag local y remotamente. Revertir la sección de `CONTRIBUTING.md` con un PR
normal. No se modifica ningún archivo fuente, manifiesto, build ni gate existente, así que una
reversión completa restaura exactamente el estado actual.

## Dependencies

- Permiso `contents: write` del repositorio para el `GITHUB_TOKEN` por defecto (creación de tag +
  Release vía API). Sin app nueva, sin PAT, sin exención de protección de ramas.
- Disciplina de Conventional Commits en `develop` — ya es la convención de facto, ahora pasa a ser
  estructural.

## Success Criteria

- [ ] Un merge a `main` produce exactamente un tag y un GitHub Release; un merge a `develop` y un PR
      abierto no producen ninguno.
- [ ] El cuerpo del Release lista los conventional commits del rango desde el tag anterior, agrupados
      por tipo, y es legible por un humano sin consultar git.
- [ ] La versión calculada sigue semver a partir de esos commits y permanece dentro de `0.x`.
- [ ] Ningún commit automatizado se envía a `main` ni a `develop`, y ninguna regla de protección de
      ramas se cambia, relaja ni evade.
- [ ] `CONTRIBUTING.md` declara el modelo de ramas, la obligación de backport de hotfix y el proceso de
      release.
- [ ] La detección de drift reporta un hotfix en `main` ausente en `develop`, y no reporta nada una vez
      que ese hotfix fue backporteado — sea por merge o por cherry-pick.
- [ ] Ninguna versión de manifiesto de crate cambia; nada se publica en crates.io.
