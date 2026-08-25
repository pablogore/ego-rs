# Propuesta: PROD-013 — Endurecimiento de la Composición de Producción

> Documento acompañante para revisión. La fuente de verdad canónica es `proposal.md` (identificadores 1:1).

## Objetivo

Una composición declarada como de producción nunca debe arrancar sobre almacenamiento
volátil porque una capacidad persistente durable no fue conectada explícitamente. Introducir
`Profile::Production` como un gate explícito de opt-in que rechaza el bootstrap — con un error
accionable — cuando cualquiera de las tres capacidades persistentes observables desde la raíz
de composición (event store, snapshot store, effect store) carece de una implementación
durable configurada explícitamente.

## Intención

`EntityRuntimeBuilder::build()` (`crates/persistent-entity/src/builder.rs:279-286`) sustituye
silenciosamente por `InMemoryEventStore` e `InMemorySnapshotStore` cuando ninguno de los dos
stores fue configurado. No hay advertencia, ni línea de log, ni camino de error — solo un
comentario de documentación de una línea ("Defaults to in-memory."). Un despliegue que se cree
listo para producción pierde entonces todos los eventos y todos los snapshots al reiniciar, y
se entera únicamente por los datos faltantes.

El effect store falla de la misma forma, no diferente: `RuntimeBuilder::with_effect_store()`
(`crates/service-sdk/src/runtime/builder.rs:501`) también tiene un fallback silencioso en
memoria — `builder.rs:811` construye `InMemoryEffectStore` cada vez que hay al menos un
effect executor registrado y ningún store fue configurado explícitamente. Un despliegue de
producción que se olvidó de conectarlo no falla de forma diferida en el primer uso; corre
silenciosamente sobre almacenamiento volátil desde el principio, exactamente igual que el
event store y el snapshot store.

PROD-012 (Idempotencia Durable, archivada) ya estableció la regla de fallar cerrado para una
capacidad persistente, el store de reservas/recibos de operación: no configurado significa
rechazado, y el rechazo nombra tanto la llamada de registro como el opt-out. PROD-013
generaliza esa regla ya establecida al resto de las capacidades persistentes de las que depende
una composición de producción, y lo hace *ahora* porque fue la propia auditoría de PROD-012 la
que dejó a la vista la brecha hermana.

El almacenamiento en memoria no es el problema y no se está eliminando. El problema es que el
almacenamiento en memoria llegue como infraestructura de producción *por default*, elegido por
omisión en lugar de por declaración.

## Decisiones Activas

| ID | Decisión | Fundamento |
|----|----------|------------|
| D-1 | El mecanismo es un enum **`Profile`** explícito de opt-in con dos variantes: `Profile::Dev` (default) y `Profile::Production`. `Profile::Dev` preserva el comportamiento actual byte por byte. `Profile::Production` exige configuración durable explícita para las capacidades dentro del alcance | Cero radio de impacto sobre los 67 call sites existentes de `EntityRuntimeBuilder::new()`; explícito y descubrible en el sitio de composición; mantiene el almacenamiento en memoria legítimamente válido para dev/test, que es para lo que existe |
| D-2 | **Revisado** — el check de "configuración parcial" no es un mecanismo separado ni corre en todo profile. Bajo `Profile::Production`, la regla de "capability faltante" (D-1) ya rechaza cualquier composición donde falte `event_store` O `snapshot_store` — cubre el caso parcial sin necesidad de una regla adicional. Bajo `Profile::Dev`, nada cambia | Se revirtió la premisa original ("cero call sites configuran esto hoy") tras verificar contra código real que existen 15 call sites en 8 archivos que configuran exactamente un store, incluida la raíz de composición de producción de reference-app (`lib.rs:502`) — un check incondicional la habría roto y habría contradicho IS-8/SC-7. Confirmado por el arquitecto; ver `design.md` AD-7 |
| D-3 | El conjunto de capacidades gateadas es **exactamente tres**: event store y snapshot store (`EntityRuntimeBuilder::build()`, `crates/persistent-entity/src/builder.rs:279-286`) y effect store (`RuntimeBuilder::with_effect_store()`, `crates/service-sdk/src/runtime/builder.rs:501`; `AppBuilder::effect_store()`, `crates/service-sdk/src/app/mod.rs:562`). Un solo mecanismo de validación cubre las tres — no tres mecanismos separados | Son las únicas capacidades persistentes que hoy tienen un slot de registro real, genérico y observable desde la raíz de composición. El effect store falla de la misma forma que las otras dos, no distinta (ver la corrección EC-2 más arriba) — un solo gate que cubra las tres es más simple que tres mecanismos separados, y ahora más preciso que el planteo original, no menos |
| D-4 | **La persistencia de read-side/proyecciones queda totalmente fuera de alcance — sin gate, real ni pseudo.** Hoy no existe ningún registro genérico de read-side en la raíz de composición: `AppBuilder::projection()` (`crates/service-sdk/src/app/mod.rs:362-391`) es inyección de dependencias de instancias de proyección ya construidas, no un slot de backend de persistencia, y el cableado real de read-side (`SharedReadSideStore` / `ReadSideSink` / `.with_read_side_sink()`) es completamente artesanal de `examples/reference-app` (explore §1.5) | PROD-013 endurece superficies de registro que existen; no va a inventar la superficie misma que se supone que debe validar solo para tener algo que gatear. Se traslada como restricción vinculante sobre PROD-014 (ver abajo) |
| D-5 | La especificación sucesora se llama **PROD-014 — Composición de Persistencia de Read-Side y Store Durable**, no solamente "Store Durable de Proyecciones de Read-Side" | Su alcance es inseparablemente dos cosas: el contrato durable (semántica de checkpoints, modelo de consistencia, esquema) Y el primer punto de registro genérico de read-side en la raíz de composición. Un nombre que cubriera solo el store habilitaría entregar la mitad |
| D-6 | Las implementaciones en memoria **no** se eliminan, ni se deprecan, ni se esconden. Siguen siendo válidas, explícitas y de primera clase para `Profile::Dev` y para los tests | El defecto es la selección silenciosa por default, no la existencia. Eliminarlas rompería el camino de dev/test que esta propuesta protege explícitamente |
| D-7 | El enfoque C (invertir el default a fallar cerrado con un opt-out nombrado, espejando `IdempotencyEnforcementMode`) está **evaluado y diferido, no rechazado por mérito** | Es el contrato de estado final más fuerte, pero rompe de inmediato ~14 archivos / ~32 call sites y exige desplegar un helper de tests estilo `compat()` en `persistent-entity`, `service-sdk` y los tests de la reference-app dentro del mismo cambio. Esa migración debe dimensionarse explícitamente en `design.md`, nunca colarse en silencio dentro de "agregar un gate" |
| D-8 | PROD-013 incluye una **segunda capa de aplicación**, no solo el flag y el error: la composición de producción de `examples/reference-app` (concretamente, vía `EntityEventStores` — ver IS-11) debe declarar explícitamente `Profile::Production`, y un check (un lint de `xtask` o un test, decidido en `design.md`) debe fallar si alguna vez deja de declararlo | El flag por sí solo es una convención descubrible, no una garantía — un host de producción todavía puede olvidarlo. Confirmado con el arquitecto: esta porción vale la superficie extra modesta (una llamada de composición más un check de regresión) en lugar de diferir la única verificación real de que el mecanismo se usa de verdad |

## Restricción de Seguimiento de Read-Side (vinculante para PROD-014)

Esta restricción se hereda textualmente de la exploración §1.5 y es un requisito sobre la
especificación sucesora, no sobre la implementación de PROD-013:

> **Restricción de seguimiento de read-side**: PROD-014 debe introducir un registro genérico de
> persistencia de read-side/proyecciones en la raíz de composición. Desde su introducción,
> Production debe aplicar la misma política de fallar cerrado que estableció PROD-013:
> capacidad no configurada → válido; capacidad configurada con un backend no durable / en
> memoria → arranque rechazado; capacidad configurada con un backend durable → válido.

*(Texto canónico en inglés, en `proposal.md`: "**Read-side follow-up constraint**: PROD-014
must introduce a generic read-side/projection persistence registration at the composition
root. From its introduction, Production must apply the same fail-closed policy PROD-013
established: capability not configured → valid; capability configured with a
non-durable/in-memory backend → startup rejected; capability configured with a durable
backend → valid.")*

El sentido de enunciarla acá, en lugar de dejarla como nota de backlog, es que PROD-014 debe
nacer honrando ya el contrato que establece PROD-013 — no ser adaptada a él después.

## Principio de Arquitectura: Regla de Completitud de Persistencia

PROD-013 documenta un principio, no solamente esta instancia del mismo:

> **Regla de completitud de persistencia** — una base de datos no se considera soportada por
> `ego-rs` hasta que implemente TODAS las capacidades persistentes que una composición de
> producción declara usar. Las capacidades faltantes no pueden completarse recayendo en
> almacenamiento en memoria. El soporte de un backend es todo-o-nada a lo largo de las
> capacidades durables que una composición habilita.

Esto es **prospectivo**. PostgreSQL es el único backend que existe hoy
(`crates/persistence/src/postgres/`), y no está en violación. La regla existe para que el
primer segundo backend parcialmente implementado sea rechazado como backend, en lugar de
entregarse como una composición de producción completada calladamente con partes en memoria.

## Límites con Especificaciones Adyacentes

| Adyacente | Su preocupación | La preocupación de PROD-013 | Solapamiento |
|-----------|-----------------|-----------------------------|--------------|
| **PROD-005** (Salud, Readiness y Arranque) | Señalizar la salud de una aplicación que **ya arrancó**, con modo degradado permitido para dependencias opcionales | **Rechazar el bootstrap mismo**, antes de que algo arranque | Ninguno. Se enuncia explícitamente para que nunca se confundan: PROD-013 decide si la app puede arrancar; PROD-005 describe una app que ya arrancó |
| **PROD-012** (Idempotencia Durable, archivada) | La misma regla de fallar cerrado para una capacidad: el store de reservas/recibos de operación | El resto de las capacidades persistentes, usando la forma de validador/error de PROD-012 como plantilla | Complementario, no solapado. PROD-013 no reabre ni modifica la regla de PROD-012 |
| **CORE-027** (`xtask verify-layers` / `verify-isolation` / `verify-hygiene`) | Dirección de dependencias entre crates (capas arquitectónicas) | Completitud del backend de persistencia en tiempo de composición | Ninguno. Se confirmó que nada existente cubre esta brecha |

## Puerta de Atomicidad (Atomicity Gate)

**Ya se corrió, y cambió el alcance — no es un paso pendiente.** La puerta identificó la
persistencia de read-side como una capacidad que habría roto la atomicidad si se absorbía: no
es un bug de fallback a endurecer, sino una capacidad faltante más una superficie de registro
faltante, e incluirla habría forzado a esta especificación a *inventar* el slot de raíz de
composición que dice validar. Absorberla se consideró y se rechazó explícitamente; se separó
hacia PROD-014 con la restricción vinculante de arriba. Las tres capacidades restantes
comparten un mecanismo, una forma de error y un criterio de aceptación, así que la
especificación es atómica tal como está delimitada.

## Alcance

### Dentro del Alcance

- **IS-1** — Introducir el enum `Profile` (`Profile::Dev` por default, `Profile::Production`) y
  el método de builder que lo establece, en la raíz de composición (D-1).
- **IS-2** — Bajo `Profile::Production`, rechazar el bootstrap cuando el **event store** no tiene
  una implementación durable configurada explícitamente, con un error que nombre la capacidad y
  `EntityRuntimeBuilder::with_event_store()`.
- **IS-3** — Bajo `Profile::Production`, rechazar el bootstrap cuando el **snapshot store** no
  tiene una implementación durable configurada explícitamente, con un error que nombre la
  capacidad y `EntityRuntimeBuilder::with_snapshot_store()`.
- **IS-4** — Bajo `Profile::Production`, rechazar el bootstrap cuando el **effect store** no tiene
  una implementación durable configurada explícitamente, con un error que nombre la capacidad y
  la llamada de configuración de la superficie de composición en uso
  (`RuntimeBuilder::with_effect_store()` o `AppBuilder::effect_store()`).
- **IS-5 (revisado)** — Eliminado como regla independiente. El caso de "exactamente uno
  configurado" queda cubierto por IS-2/IS-3 bajo `Profile::Production` — el gate ya rechaza
  cualquier store faltante, parcial o total. Ver AD-7 en `design.md`.
- **IS-6** — Un único validador como fuente de verdad de la regla para las tres capacidades,
  siguiendo la forma de `validate_idempotency()` de PROD-012
  (`crates/service-sdk/src/runtime/builder.rs:735-771`).
- **IS-7** — Los errores son accionables: cada uno nombra la capacidad faltante Y la llamada de
  configuración exacta que lo corrige, en el estilo que afirma el test de PROD-012
  `the_refusal_names_the_registration_and_the_opt_out`.
- **IS-8** — Preservar sin cambios el comportamiento actual para toda composición que no
  establezca `Profile::Production` — los 67 call sites existentes de
  `EntityRuntimeBuilder::new()` siguen compilando y pasando sin modificación.
- **IS-9** — Documentar la **regla de completitud de persistencia** como principio de
  arquitectura, en carácter prospectivo y no como reporte de una violación actual.
- **IS-10** — Documentar explícitamente el límite con PROD-005 (rechazo del bootstrap frente a
  salud posterior al arranque), para que las dos especificaciones nunca se confundan.
- **IS-11 (revisado)** — El profile viaja en el tipo `EntityEventStores`, no como parámetro de
  `build_runtime_with` (que es compartido entre dev y producción). `EntityEventStores::open(pool)`
  produce `Profile::Production`; `EntityEventStores::in_memory()` produce `Profile::Dev`. El
  campo profile es privado — esos dos constructores son la única forma de establecerlo, así que
  un store durable y una declaración de producción no pueden desincronizarse. Esto cierra R-1 de
  forma estructural, más fuerte que la garantía original de "una llamada más un check de
  regresión". Ver `design.md` AD-8.
- **IS-12** — Un check (un lint de `xtask` o un test — el mecanismo exacto es una decisión de
  `design.md`) hace fallar el build si la composición de producción de reference-app (vía
  `EntityEventStores`, consumido por `build_runtime_with`) alguna vez deja de declarar
  `Profile::Production`, así esta referencia sigue siendo un guardia de regresión vivo, no un
  ejemplo de una sola vez (D-8).
- **IS-13 (nuevo, por AD-9)** — `EntityEventStores::open(pool)` conecta también dos instancias
  reales de `PostgreSQLSnapshotStore` (uno por agregado: organización y usuario), reemplazando
  el snapshot store en memoria que la ruta de producción de reference-app usa hoy silenciosamente.
  Esto es cablear, no construir un backend nuevo — `PostgreSQLSnapshotStore` ya existe
  (`crates/persistence/src/postgres/snapshot.rs`) — y cierra un defecto de durabilidad real que
  el propio gate de PROD-013 expone en su host de referencia: si no se conecta, el gate
  rechazaría la propia composición de producción de reference-app por falta de snapshot store.
  Riesgo conocido para tasks/apply: `PostgreSQLSnapshotStore::save_snapshot` usa
  `tokio::task::block_in_place`, que panickea en un runtime Tokio de un solo hilo — cualquier
  test de integración Postgres que pueda disparar un snapshot real (más de 100 eventos, el
  umbral de `PeriodicSnapshotStrategy`) debe usar `#[tokio::test(flavor = "multi_thread")]`.

### Fuera del Alcance

- **OOS-1** — Construir cualquier backend nuevo de Postgres. El event store durable, el snapshot
  store durable (`crates/persistence/src/postgres/{event_store,snapshot}.rs`) y el effect store
  durable (vía PROD-002) ya existen. PROD-013 valida configuración, no implementa
  almacenamiento. OOS-1 sigue excluyendo construir un backend nuevo — conectar
  `PostgreSQLSnapshotStore` (ya existente) en la ruta de producción de reference-app (IS-13) es
  cableado, no implementación, y por lo tanto es coherente con OOS-1, no una excepción a él.
- **OOS-2** — Persistencia de read-side / proyecciones / checkpoints, y cualquier gate de
  read-side, real o pseudo (D-4). Diferido a PROD-014 con la restricción vinculante de arriba.
- **OOS-3** — Observabilidad, alta disponibilidad, migraciones y todo otro tema de endurecimiento
  de producción. No se mezclan en esta especificación atómica.
- **OOS-4** — Soporte para un segundo motor de base de datos (Oracle, MySQL, SQLite u otro). Hoy
  no existe ninguno; la regla de completitud de persistencia es prospectiva y todavía no tiene
  nada contra lo que validar.
- **OOS-5** — Decidir el enfoque C (inversión del default con un opt-out nombrado) y su migración
  de ~32 call sites. Se registra en `design.md` como alternativa evaluada, diferida acá (D-7), no
  resuelta en esta propuesta.
- **OOS-6** — Eliminar, deprecar u ocultar las implementaciones en memoria (D-6).
- **OOS-7** — Reabrir la regla de idempotencia de PROD-012 o su modo de aplicación.
- **OOS-8** — Patrones de outbox/inbox. Se confirmó que están completamente ausentes del código;
  no aplica.

## Capacidades

### Capacidades Nuevas

- `production-composition-hardening`: validación de fallar cerrado, gateada por profile, de las
  capacidades persistentes durables en la raíz de composición, más la regla de completitud de
  persistencia.

### Capacidades Modificadas

- `persistent-entity`: `EntityRuntimeBuilder::build()` ya no sustituye silenciosamente por
  event/snapshot stores en memoria bajo `Profile::Production` — la misma regla de capability
  faltante rechaza un store faltante sin importar si falta solo uno o ambos (AD-7). No existe
  una verificación de configuración parcial separada, y `Profile::Dev` no se ve afectado.
- `application-composition`: la raíz de composición gana una declaración explícita de profile, y
  `AppBuilder`/`RuntimeBuilder` exponen el rechazo del gate a través del camino de error de
  composición existente.

## Enfoque

Reutilizar la forma ya probada de PROD-012 en lugar de inventar un segundo mecanismo: un enum
cuyo default no cambia nada, un validador privado como única fuente de verdad de la regla, y un
error cuyo mensaje nombra tanto la capacidad faltante como la llamada que lo corrige. El
validador verifica las tres capacidades en un solo lugar, así que hay una regla que entender,
una forma de error que testear y una sola cosa que extender cuando PROD-014 agregue read-side.

Dos restricciones estructurales determinan dónde viven las piezas. Primero, `persistent-entity`
**no tiene dependencia sobre `service-sdk`** (explore §1.3), mientras que `service-sdk` sí
depende de `persistent-entity` — así que un único tipo `Profile` compartido debe declararse en el
crate inferior y reexportarse hacia arriba, y el error del gate de event/snapshot debe definirse
localmente en `persistent-entity`, cruzando el límite de capas exactamente como ya lo hace
`RuntimeError::OperationReservationStoreNotRegistered` una capa más arriba. Segundo,
`EntityRuntimeBuilder::build()` es infalible (`-> EntityRuntime<E>`) y no tiene un hermano
`try_build()`, a diferencia de `RuntimeBuilder` — así que cómo se manifiesta ahí el rechazo (un
hermano falible, o el ordenamiento validar-antes-de-delegar de PROD-012) es una decisión de
`design.md`, deliberadamente no pre-comprometida acá. `CompositionError` ya lleva
`Validation(#[from] RuntimeError)`, de modo que una variante nueva de `RuntimeError` se
manifiesta a través de `AppBuilder` sin costo adicional.

El caso de configuración parcial (D-2, revisado) no es una verificación separada: bajo
`Profile::Production` ya queda atrapado por la regla de capability faltante (cualquiera sea el
store que falte, faltar es faltar), y bajo `Profile::Dev` sigue siendo válido, sin cambios. Se
consideró una verificación independiente del profile y se revirtió al comprobar contra call
sites reales que no era gratis — ver `design.md` AD-7.

## Criterio de Aceptación

```
Dada una composición configurada con Profile::Production
Cuando alguna de {event_store, snapshot_store, effect_store} no tiene una
      implementación durable explícitamente configurada
Entonces el bootstrap DEBE rechazarse con un error accionable que nombre la
      capacidad faltante y su llamada de configuración correspondiente
      (EntityRuntimeBuilder::with_event_store,
       EntityRuntimeBuilder::with_snapshot_store,
       RuntimeBuilder::with_effect_store / AppBuilder::effect_store)
Y NUNCA debe degradar silenciosamente a in-memory
      (InMemoryEventStore / InMemorySnapshotStore).

Dada una composición SIN Profile::Production (dev/test, el default)
Cuando ninguna capacidad está configurada
Entonces el comportamiento actual (in-memory) se preserva sin cambios, y
      EntityRuntimeBuilder::build() sigue teniendo éxito.

Dada una composición configurada con Profile::Production
Cuando se configura event_store O snapshot_store pero no ambos
Entonces el bootstrap DEBE rechazarse (subsumido por la misma regla del caso totalmente
        faltante, no un mecanismo separado).

Dada una composición SIN Profile::Production (dev/test)
Cuando se configura event_store O snapshot_store pero no ambos
Entonces el comportamiento actual se preserva — la configuración parcial sigue siendo
        válida, sin cambios.
```

## Áreas Afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistent-entity/src/builder.rs:279-286` | Modificado | Los dos fallbacks `unwrap_or_else` a memoria pasan a estar gateados bajo `Profile::Production`; la misma regla de capability faltante subsume la configuración parcial, sin verificación separada (AD-7) |
| `crates/persistent-entity` (tipo `Profile` + error local del gate) | Nuevo | `Profile` declarado en el crate inferior (no hay dependencia disponible a `service-sdk`) más un tipo de error local para el rechazo de event/snapshot |
| `crates/service-sdk/src/runtime/builder.rs` (`with_effect_store` en :501, patrón de validador en :735-771) | Modificado | El effect store gana el mismo gate; la forma del validador de PROD-012 es la plantilla |
| `crates/service-sdk/src/app/mod.rs` (`effect_store` en :562, `build()` en :791) | Modificado | Declaración de profile en la raíz de composición de `AppBuilder`; el rechazo se manifiesta vía `CompositionError::Validation` |
| `crates/persistence/src/postgres/` | Sin cambios | Las implementaciones durables ya existen; acá no se construye nada (OOS-1) |
| `crates/infrastructure/src/persistence/in_memory/` | Sin cambios | Las implementaciones en memoria siguen siendo válidas y explícitas para `Profile::Dev` (D-6) |
| ~14 archivos / ~32 call sites que dependen del default silencioso | Sin cambios | Nunca establecen `Profile::Production`, así que `Profile::Dev` preserva su comportamiento (IS-8) |
| `AppBuilder::projection()` (`app/mod.rs:362-391`), `SharedReadSideStore` / `ReadSideSink`, cableado de read-side de `examples/reference-app` | Intocado | Sin ningún gate de read-side (D-4, OOS-2) |
| `examples/reference-app/src/lib.rs` (`build_runtime_with` en :567, `build_runtime_in_memory` en :311, `build_runtime_observed_in_memory` en :522, `EntityEventStores`) | Modificado | `EntityEventStores::open` porta `Profile::Production` y ahora también conecta dos instancias de `PostgreSQLSnapshotStore` (org + usuario); `EntityEventStores::in_memory()` sigue en `Profile::Dev` (D-8, IS-11, IS-13) |
| Nuevo check de regresión (lint de `xtask` o test, mecanismo decidido en `design.md`) | Nuevo | Hace fallar el build si la composición de producción de reference-app (vía `EntityEventStores`) deja de declarar `Profile::Production` (D-8, IS-12) |
| Tests de integración Postgres que puedan disparar un snapshot real (p. ej. `durable_entity_progress_postgres.rs` y afines) | Riesgo | `PostgreSQLSnapshotStore::save_snapshot` usa `tokio::task::block_in_place`, que panickea en un runtime de un solo hilo; esos tests deben usar `#[tokio::test(flavor = "multi_thread")]` (IS-13) |
| Documentación de arquitectura | Modificado | Regla de completitud de persistencia (IS-9) y límite con PROD-005 (IS-10) |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| R-1 | Hay que *acordarse* de `Profile::Production`. Un host de producción que se olvide del flag obtiene exactamente la clase de fallo que esta especificación existe para cerrar, movida un nivel más arriba (de "se olvidó el store" a "se olvidó el profile") | Alta | **Resuelto (D-8).** La composición de producción de `examples/reference-app` declara `Profile::Production` explícitamente, y un check hace fallar el build si esa declaración alguna vez regresa — la referencia sigue siendo un guardia vivo, no un ejemplo de una sola vez. Esto no protege a un host *distinto* que nunca mira a reference-app, lo que queda como riesgo residual aceptado de toda convención de opt-in |
| R-2 | El inventario de radio de impacto (explore §2: 67 call sites / 25 archivos; ~32 sitios / 14 archivos sobre el default silencioso) se derivó buscando llamadas de override de store **colocadas en el mismo archivo**. Un call site que configure su store mediante un fixture compartido en otro archivo no aparecería | Media | Re-verificar durante design/apply antes de confiar en el conteo. Que `Profile::Dev` sea el default hace que un inventario incompleto sea no-rompiente en lugar de una regresión |
| R-3 | `EntityRuntimeBuilder::build()` es infalible y no tiene hermano `try_build()`, así que el rechazo no tiene ningún camino `Result` existente por donde viajar | Media | Decisión explícita de `design.md`. El ordenamiento validar-antes-de-delegar de PROD-012 es la referencia; ese ordenamiento es estructural (la validación debe correr antes de delegar, o el panic desenrolla antes de que el camino `Result` pueda devolverlo) |
| R-4 | Límite de capas: `persistent-entity` no puede tomar prestado `RuntimeError`/`CompositionError`, así que se introduce un segundo tipo de error en el crate inferior | Baja | Existe precedente — `RuntimeError::OperationReservationStoreNotRegistered` cruza el mismo límite una capa más arriba. `CompositionError::Validation(#[from] RuntimeError)` ya reenvía |
| R-5 | Expansión de alcance: la inclusión del effect store invita a "endurecer todas las capacidades", o la regla de completitud de persistencia invita a trabajo de segundo backend | Media | D-3 fija el conjunto en exactamente tres; OOS-2/OOS-4 cierran read-side y multi-backend. La regla de completitud se entrega solo como principio documentado, sin nada contra lo que validar hoy |
| R-6 | La regla de completitud de persistencia se lee como reporte de una violación actual y dispara trabajo innecesario de Postgres | Baja | IS-9 la enuncia como prospectiva. PostgreSQL es el único backend hoy y no está en violación |
| R-7 | Si el enfoque C se adopta más adelante, el despliegue del helper de tests estilo `compat()` en ~14 archivos es por sí mismo no trivial | Media | D-7/OOS-5: se dimensiona explícitamente como su propia unidad de trabajo en `design.md`, nunca colado en silencio dentro de "agregar el gate" |

## Plan de Reversión

El cambio es aditivo e inerte por default. `Profile::Dev` es el default y reproduce exactamente
el comportamiento actual, así que revertir consiste en quitar el enum `Profile`, su método de
builder, el validador y las variantes de error nuevas; todos los call sites existentes quedan
intocados tanto por el cambio como por la reversión. No hay un comportamiento de configuración
parcial separado que revertir — el caso parcial queda subsumido por la regla de capability
faltante bajo `Profile::Production` (D-2, revisado; AD-7), y `Profile::Dev` nunca lo restringió.
No se afecta ningún esquema, ninguna migración, ningún dato ni ningún formato de almacenamiento en
runtime: PROD-013 no escribe código de persistencia, solo de validación. Los cambios de
documentación (IS-9, IS-10) se revierten en un solo commit.

## Dependencias

- PROD-012 (Idempotencia Durable, archivada) — aporta la plantilla de validador/error
  (`crates/service-sdk/src/runtime/builder.rs:735-771`). Se reutiliza, no se modifica.
- PROD-002 (archivada) — el effect store durable contra el que PROD-013 gatea ya existe.
- El event store y el snapshot store durables existentes en Postgres
  (`crates/persistence/src/postgres/{event_store,snapshot}.rs`).
- Ninguna dependencia externa, crate, servicio o infraestructura nueva.

## Criterios de Éxito

- [ ] **SC-1** — Una composición con `Profile::Production` y sin event store configurado es
      rechazada en el bootstrap, y el error nombra tanto la capacidad como
      `EntityRuntimeBuilder::with_event_store()`.
- [ ] **SC-2** — Lo mismo vale para el snapshot store, nombrando
      `EntityRuntimeBuilder::with_snapshot_store()`.
- [ ] **SC-3 (corregido)** — Lo mismo vale para el effect store, nombrando la llamada de
      configuración de la superficie en uso. El effect store SÍ tiene un fallback silencioso
      real a `InMemoryEffectStore` (`crates/service-sdk/src/runtime/builder.rs:811`, hallazgo
      posterior a la exploración inicial, que solo buscó el patrón `unwrap_or_else` y no vio
      este `match`) — el defecto es el mismo tipo de volatilidad silenciosa que event/snapshot
      store, no una falla diferida al primer uso. El gate solo aplica cuando hay al menos un
      effect executor registrado (si no hay ninguno, no se construye ningún store, nada es
      volátil).
- [ ] **SC-4** — Bajo `Profile::Production`, ningún camino de código puede alcanzar
      `InMemoryEventStore` ni `InMemorySnapshotStore` por default; el fallback silencioso
      `unwrap_or_else` es inalcanzable.
- [ ] **SC-5** — Una composición sin `Profile::Production` y sin nada configurado sigue
      construyéndose con éxito sobre almacenamiento en memoria, con comportamiento idéntico al
      actual.
- [ ] **SC-6 (revisado)** — Configurar exactamente uno de `{event_store, snapshot_store}` bajo
      `Profile::Production` es rechazado (por IS-2/IS-3, no por una regla separada). Bajo
      `Profile::Dev`, configurar parcialmente sigue siendo válido, sin cambios — es el
      comportamiento actual, y romperlo violaría IS-8/SC-7.
- [ ] **SC-7** — Los 67 call sites existentes de `EntityRuntimeBuilder::new()` compilan y pasan
      sin modificación; `cargo test --workspace` no muestra fallos nuevos.
- [ ] **SC-8** — Un único validador es la fuente de verdad de la regla para las tres capacidades;
      no existe una segunda verificación paralela.
- [ ] **SC-9** — No existe ningún gate de read-side/proyecciones en ninguna parte del cambio, y la
      restricción de seguimiento de read-side queda registrada textualmente contra PROD-014.
- [ ] **SC-10** — La regla de completitud de persistencia y el límite con PROD-005 están
      documentados y son legibles por una persona, no meramente parseables por un agente.
- [ ] **SC-11 (revisado)** — `EntityEventStores::open(pool)` de `examples/reference-app` produce
      `Profile::Production` (consumido por `build_runtime_with`, compartido entre dev y
      producción); `EntityEventStores::in_memory()` produce `Profile::Dev`. El campo profile es
      privado, así que el check de que esa declaración se elimine o debilite se reduce a
      afirmar `stores.profile()` en ambos constructores (AD-10).
