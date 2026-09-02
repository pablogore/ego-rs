# Propuesta: PROD-014A — Composición del Progreso Durable del Read-Side

> Documento acompañante para revisión. La fuente de verdad canónica es `proposal.md` (identificadores 1:1).

## Objetivo

Una composición declarada como `Profile::Production` no debe poder arrancar una proyección de
read-side cuyo estado de progreso durable — su `OffsetStore` y su `DedupStore`, el par que
decide si una proyección puede retomar correctamente tras un reinicio — sea volátil. Darle al
progreso del read-side su primer punto de registro en la raíz de composición, clasificarlo con
el mecanismo `is_durable()` ya establecido por PROD-013, y rechazar el bootstrap en
`AppBuilder::build()` cuando se declara Production y ese par no es durable.

## Intención

El progreso del read-side es hoy la **única** capability persistente de este workspace sin
ninguna visibilidad en tiempo de composición. No está "sin guardia"; es inobservable.

- `AppBuilder`, `RuntimeBuilder` y `App` no tienen ninguna referencia a `OffsetStore`,
  `DedupStore` ni a ningún cableado de read-side.
  `RuntimeBuilder::validate_persistence_profile()`
  (`crates/service-sdk/src/runtime/builder.rs:777`) verifica el event store, el snapshot store
  y el effect store, y nada más. No existe ningún camino de código que siquiera pudiera
  observar la discrepancia.
- `ReadSideHandles::new()` (`examples/reference-app/src/read_side/mod.rs:103-113`) construye
  incondicionalmente `InMemoryOffsetStore` / `InMemoryDedupStore` sin parámetro, sin punto de
  inyección y sin ninguna decisión visible en composición para el host. Esa invisibilidad es
  exactamente lo que debe desaparecer del camino de Production.
- Por lo tanto una composición puede declarar `Profile::Production`, pasar todas las puertas de
  PROD-013, y levantar un pipeline de read-side totalmente volátil sin rechazo, sin advertencia
  y sin una línea de log. Al reiniciar, cada proyección retoma desde
  `read_offset() -> Ok(None)` y reprocesa el stream completo sin memoria de deduplicación.

PROD-013 cerró esta clase de falla para el event store, el snapshot store y el effect store, y
dejó registrada una restricción vinculante sobre su sucesor: *"PROD-014 debe introducir un
registro genérico de persistencia de read-side/proyecciones en la raíz de composición. Desde su
introducción, Production debe aplicar la misma política fail-closed que PROD-013 estableció."*
PROD-014A cumple exactamente esa restricción y nada más.

Los offset/dedup stores en memoria no son el problema y no se eliminan. El problema es que una
composición de producción reciba estado de resumen volátil **por silencio** en lugar de por
declaración.

## Decisiones Activas

| ID | Decisión | Justificación |
|----|----------|---------------|
| D-1 | El cambio se titula **PROD-014A — Composición del Progreso Durable del Read-Side**, no "Read-Side Persistence Composition" como estaba reservado en ROADMAP.md §7.13 / PROD-013 D-5. El slug de la carpeta de cambio y el topic key siguen siendo `prod-014a-read-side-persistence-composition` (sin cambios, para no generar churn de artefactos) | La exploración demostró que la capability realmente en alcance es el **estado de progreso durable** — `OffsetStore` + `DedupStore`, el par que permite a una proyección retomar correctamente — y no el SPI genérico huérfano `ProjectionStateStore` que asumía el brief original (D-4). "Persistence Composition" invita a gobernar cualquier tipo de read-side que quede cerca; "Composición del Progreso Durable" nombra la cosa real que se compone y se gobierna, y vuelve legible OOS-8 (`ReadSideStore` mismo) como un límite y no como una omisión. Esto refleja PROD-015 D-1, que igualmente registró una desviación deliberada de nombre respecto de ROADMAP.md en lugar de renombrar en silencio |
| D-2 | **Se adopta el enfoque A2 (registro en la raíz de composición + puerta en `build()`); se rechaza A1 (pasar `Profile` a `ProjectionSpec::new` / `TagSchedulerImpl::spawn` y rechazar en spawn).** El razonamiento del arquitecto, registrado íntegro: (a) `ProjectionSpec` y `TagSchedulerImpl::spawn()` son superficies de **ejecución** del read-side, no superficies de composición ni de seguridad de despliegue — introducir `Profile` allí filtraría una preocupación de despliegue/seguridad de composición dentro del scheduler solo para minimizar el diff; (b) con A1, `AppBuilder::build()` podría tener éxito bajo `Profile::Production` y el read-side solo podría descubrirse inválido más tarde, en tiempo de `spawn()`, lo que debilitaría el significado que PROD-013 ya estableció para `Profile::Production` — una composición incorrecta nunca debe poder arrancar, así que el rechazo debe ocurrir en composición/bootstrap (`build()`), nunca diferido a `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()` ni al primer batch; (c) A2 además corrige el defecto que la exploración expuso — hoy `ReadSideHandles::new()` construye silenciosamente `InMemoryOffsetStore`/`InMemoryDedupStore` sin ninguna decisión visible en composición para el host, y esa invisibilidad es exactamente lo que debe desaparecer del camino de Production; (d) `AppBuilder` ya es la raíz de composición explícita orientada a la aplicación que delega cada registro a `RuntimeBuilder` — el precedente exacto es `.effect_store()` → `RuntimeBuilder::with_effect_store()` — de modo que A2 sigue la forma existente en vez de inventar una; (e) críticamente, **A2 no significa que `AppBuilder`/`ego-rs` construya los stores** — el non-goal de CORE-026 permanece plenamente intacto (D-5) | A1 compra un diff más chico reubicando la garantía en la capa equivocada. Una puerta que dispara después de que `build()` ya tuvo éxito es un contrato distinto y más débil que el que PROD-013 entregó, y ambos quedarían en desacuerdo sobre qué significa `Profile::Production` |
| D-3 | **El registro es por proyección, con clave `projection_id` — no un único slot global.** Investigado antes de decidir, contra el modelo de ejecución real: (1) `TagSchedulerImpl::spawn(self, spec)` (`crates/runtime/src/read_side/scheduler.rs:276-277`) consume el scheduler por valor, así que un `TagSchedulerImpl` produce exactamente un poll loop y N proyecciones requieren N instancias de scheduler — nada limita N; (2) `ProjectionSpec<F, H, S, D, O, R>` (`:175`) lleva `dedup_store: D` y `offset_store: O` como **parámetros genéricos por instancia**, de modo que dos proyecciones pueden legítimamente usar *tipos* concretos distintos de store, algo que un único slot global borrado no podría representar; (3) ambos espacios de clave ya están namespaced por proyección: `OffsetKey = (projection_id, tag, tenant)` y `DedupKey = (projection_id, tag, event_id)` (`examples/reference-app/src/read_side/store.rs:163-167, 209-213`), así que el registro por proyección también *permite* compartir una única instancia entre proyecciones sin colisión — es estrictamente la forma más permisiva, y no cuesta nada cuando N=1; (4) `AppBuilder` ya tiene precedente de registro con multiplicidad por clave en `.projection()` y `.entity()` (dup-guarded por clave), así que no es una forma novedosa de builder; (5) reference-app corre hoy exactamente una proyección (`PROJECTION_ID = "users-by-tenant"`), así que N=1 es el caso degenerado de la forma elegida, no un caso especial que necesite diseño propio | El argumento decisivo es el **sujeto** de la puerta. `validate_persistence_profile` puede saltear el effect store cuando `effect_executors.is_empty()` porque el registro de executors hace visible en composición la existencia de la capability. El read-side hoy no tiene esa señal. Un slot global único le daría a la puerta un par anónimo y seguiría sin responder "¿esta proyección corre sobre progreso durable?"; usar `projection_id` como clave le da a la puerta un sujeto real por proyección y convierte "cero registradas = no hay read-side = válido" en un hecho y no en una suposición. Por eso la forma de slot único de `.effect_store()` se reutiliza solo como **mecanismo de validación y de duplicado fail-closed**, nunca como forma estructural |
| D-4 | **`ProjectionStateStore` queda totalmente excluido de este cambio** y se documenta como un fragmento desconectado/abandonado de CORE-005 | Evidencia de la exploración: cero implementaciones y cero llamadores en todo el workspace; su único consumidor plausible `ReadSideProcessor` también tiene cero implementaciones; un grep de la cadena literal sobre todo OpenSpec no devuelve ningún hit fuera de la propia exploración; la spec, el tasks.md, el data-model.md y el contracts/README.md de CORE-005 definen la persistencia de estado del read-side puramente como `OffsetStore` + `DedupStore`, nunca como un store dedicado de `ProjectionState`. Gobernarlo endurecería un puerto muerto y no cerraría ninguna brecha real de producción. Eliminarlo o reubicarlo es trabajo de higiene separado (F-3), no este cambio |
| D-5 | **El delta a CORE-026 es una clarificación de límite, no una renegociación.** Dos ejes ortogonales se enuncian explícitamente en el delta: "el framework construye o define por defecto stores de read-side" — **sigue siendo un non-goal, sin cambios**; "la raíz de composición acepta, clasifica y valida un par construido por el host" — **nuevo, en alcance** | Los Non-Goals de CORE-026 (`openspec/specs/read-side/spec.md:160-171`) rechazan una conveniencia del framework que *construya internamente* dedup/offset stores, porque el handler y el closure de descubrimiento de tags son irreduciblemente específicos de la aplicación. Nada en ese razonamiento aborda inspeccionar una propiedad de durabilidad de un store que la aplicación ya construyó y entregó. Ese non-goal además es anterior a `Profile::Production` (PROD-013), así que nunca abordó este eje — no está obsoleto, es ortogonal. El delta igualmente es necesario, porque el mismo texto de Non-Goals afirma que esta capability "envuelve el contrato existente del motor, no renegocia ninguna parte de él", y un nuevo rechazo en tiempo de composición es un cambio real (estrecho, aditivo) en el comportamiento observable que debe especificarse y no agregarse en silencio |
| D-6 | Un **par durable falso, solo de test** (`is_durable() -> true`) alcanza para demostrar el camino de aceptación de Production. No se construye ningún backend durable real | No existe ningún `OffsetStore`, `DedupStore` ni `ReadSideStore` durable en ningún lugar del workspace (`crates/persistence/src/postgres/` tiene event store, snapshot, repository, reservation — nada de read-side). Construir uno es una brecha de implementación separada con su propio tamaño (F-1). Incluirla rompería la Puerta de Atomicidad, y la capability de gobierno es testeable y útil sin ella |
| D-7 | **`ReadSideStore` (la fuente de eventos que la proyección consulta) no queda gobernado por este cambio** (OOS-8) | Es una vista de lectura del stream de eventos, no estado de resumen. Su contenido deriva del event store aguas arriba, cuya durabilidad PROD-013 ya gobierna. Gobernarlo exigiría decidir qué es siquiera una vista de eventos de read-side durable — una pregunta materialmente distinta de "¿puede esta proyección retomar?". Se nombra como límite, con F-4 como seguimiento |
| D-8 | La durabilidad se declara **únicamente** vía `is_durable()` en los dos SPIs más `require_durably_configured(...)`, reutilizado verbatim y con su firma existente | El mecanismo de PROD-013, sin cambios. Sin `TypeId`, sin downcasting, sin coincidencia por nombre de tipo, sin heurística. El propio doc comment de `require_durably_configured` ya prohíbe computar su argumento `durably_configured` a partir de `.is_some()`; los call sites de este cambio lo computan desde `is_durable()` en ambos stores |

## Puerta de Atomicidad

**Ejecutada, y recortó el alcance dos veces.** Un backend durable real de read-side en Postgres
fue considerado y removido (D-6 → F-1): es una implementación entregable de forma independiente
con su propio esquema, migración y obligaciones de conformidad, y la capability de esta
propuesta es testeable sin ella. Eliminar el fragmento abandonado `ProjectionStateStore` /
`ReadSideProcessor` fue considerado y removido (D-4 → F-3): es higiene de borrado sin
dependencia con esta puerta en ninguna dirección.

Lo que queda es una capability indivisible, porque ningún ítem en alcance es entregable de
forma independiente con valor:

- IS-1 por sí solo es un `is_durable()` que nadie llama — código muerto.
- IS-2 por sí solo es un slot de registro que nada valida — peor que ausente, porque *parece*
  gobierno.
- IS-3/IS-4/IS-5 no pueden existir sin IS-1 (el hecho) e IS-2 (el sujeto).
- IS-7 es lo que hace que IS-2 no sea decorativo: si el camino de Production del host de
  referencia no obtiene su par desde la composición, un host puede registrar un par durable y
  entregarle uno volátil a `ProjectionSpec`, y la puerta pasaría sobre una proyección volátil.
- IS-8 es la única forma de ejercitar la rama de aceptación, dado D-6.

Cada ítem nombra el mismo mecanismo (`is_durable()` + `require_durably_configured`), la misma
forma de error (`PersistenceCompositionError::NotConfigured { capability, fix }` expuesta a
través de `CompositionError::Validation`) y el mismo criterio de aceptación.

**ATOMICITY: PASS**

## Alcance

### En Alcance

- **IS-1** — Agregar `is_durable(&self) -> bool { false }` como método por defecto en
  `OffsetStore` (`crates/domain/src/read_side/offset.rs`) y `DedupStore`
  (`crates/domain/src/read_side/dedup.rs`), reflejando el idiom de `EventStore` / `Snapshot`
  que estableció PROD-013. El default `false` mantiene compilando y honesta a toda
  implementación existente.
- **IS-2** — Un punto de registro en la raíz de composición para el par de progreso durable de
  una proyección (`OffsetStore` + `DedupStore` juntos), con clave `projection_id` (D-3). El par
  es la unidad: un registro que cubra solo uno de los dos NO DEBE ser representable, de modo que
  una configuración parcial nunca pueda pasar la validación como si ambos estuvieran cubiertos.
  La superficie pública exacta — dos métodos, un `read_side_store(...)` /
  `read_side_persistence(...)`, una struct de registro, u otra cosa — es una decisión de
  `design.md` derivada de estos invariantes, no fijada aquí.
- **IS-3** — Un registro duplicado para el mismo `projection_id` falla cerrado en `build()` con
  un error de composición que nombra el duplicado, nunca last-write-wins — siguiendo la forma
  de `pending_error` latcheado de `AppBuilder::effect_store()` y el requisito
  "Duplicate Effect Store Registration Through AppBuilder Fails Closed".
- **IS-4** — Bajo `Profile::Production`, `AppBuilder::build()` (a través de
  `RuntimeBuilder::try_build()` → `validate_persistence_profile()` →
  `CompositionError::Validation`, el camino exacto que PROD-013 ya usa) rechaza el bootstrap
  cuando el `OffsetStore` o el `DedupStore` de una proyección registrada no es durable. El error
  nombra la capability faltante y la llamada exacta que lo corrige.
- **IS-5** — Una composición `Profile::Production` **sin** read-side registrado construye
  exitosamente. Las aplicaciones solo de comandos o sin read-side nunca son forzadas a
  registrar un store dummy, reflejando la condicionalidad del effect store: "sin executor
  registrado no hay nada volátil que rechazar".
- **IS-6** — `Profile::Dev` no cambia: los offset/dedup stores volátiles en memoria siguen
  siendo válidos, explícitos y de primera clase. Todo call site existente compila y pasa sin
  modificación.
- **IS-7** — El camino de composición de Production de `examples/reference-app` obtiene su par
  offset/dedup desde la raíz de composición en vez de que `ReadSideHandles::new()` construya
  `InMemoryOffsetStore` / `InMemoryDedupStore` por su cuenta. Los caminos de Dev y test pueden
  seguir construyéndolos explícitamente. El mecanismo que evita que el host de referencia
  regrese en silencio es una decisión de `design.md`; el acoplamiento estructural
  `EntityEventStores::open()` / `::in_memory()` de PROD-013 (IS-11/IS-12) es el precedente de
  referencia, no un mandato.
- **IS-8** — Un par durable falso solo de test (`is_durable() -> true`) que demuestra el camino
  de aceptación de Production (D-6).
- **IS-9** — Deltas de spec: una clarificación de límite más el nuevo requisito de composición
  en `read-side` (D-5), el requisito de registro y de duplicado fail-closed en
  `application-composition`, y la extensión de la puerta de Production en
  `production-composition-hardening`.
- **IS-10** — Corregir el doc comment de `Profile::Production`
  (`crates/persistent-entity/src/profile.rs:17-23`), que hoy afirma que el read-side
  "no tiene tal slot todavía y deliberadamente no se gobierna aquí". Una vez que IS-2 aterrice
  esa frase es falsa, y además nombra el alcance sucesor equivocado.

### Fuera de Alcance

- **OOS-1** — `ProjectionStateStore` y `ReadSideProcessor`. Intocados, en ambas direcciones
  (D-4). Su eliminación o reubicación es F-3.
- **OOS-2** — Cualquier backend durable real: `PostgreSQLOffsetStore`, `PostgreSQLDedupStore` o
  un `ReadSideStore` durable. Reservado para **un futuro cambio PROD-014B o equivalente de store
  durable de read-side en postgres** — el identificador se deja deliberadamente abierto aquí en
  lugar de comprometerse en firme (F-1). Ese cambio debe existir; de lo contrario la puerta de
  PROD-014A es un rechazo sin nada en el árbol que lo satisfaga.
- **OOS-3** — Introducir `Profile` en `ProjectionSpec`, `TagSchedulerImpl`, `ReadSideSession` o
  `ReadSideRunner` (D-2). Ningún cambio en la semántica de polling, dedup, offset u orden.
- **OOS-4** — Ownership multi-worker, fencing, leasing de particiones, HA, brokers/Kafka,
  entrega exactly-once y orquestación de rebuild de proyecciones.
- **OOS-5** — Cualquier cambio a los contratos existentes de CORE-007 o CORE-028 más allá de la
  superficie aditiva de registro en sí.
- **OOS-6** — Que el framework construya o defina por defecto stores de read-side. El non-goal
  de CORE-026, intacto y sin afectar (D-5).
- **OOS-7** — Gobernar una proyección levantada enteramente fuera de la raíz de composición. Un
  host todavía puede llamar `ProjectionSpec::new(...)` y `spawn(...)` directamente con sus
  propios stores; ese camino queda no gobernado por construcción (D-2 rechaza el único mecanismo
  que lo cerraría). Riesgo residual R-1.
- **OOS-8** — Durabilidad de `ReadSideStore` (D-7). Seguimiento F-4.
- **OOS-9** — Eliminar, deprecar u ocultar `InMemoryOffsetStore` / `InMemoryDedupStore`.
  Siguen siendo válidos y explícitos para Dev y tests (IS-6).

## Capabilities

### Capabilities Nuevas

- Ninguna. Este cambio extiende tres capabilities existentes en vez de introducir una cuarta
  superficie para la misma regla.

### Capabilities Modificadas

- `read-side`: la clarificación de límite que exige D-5 (la construcción por parte del framework
  sigue fuera de alcance; la aceptación y validación en la raíz de composición de un par
  construido por el host es nueva), más la afirmación de que el par de progreso durable de una
  proyección puede componerse en la raíz de composición y ser rechazado allí bajo Production.
- `application-composition`: el registro del par de progreso durable de una proyección con clave
  `projection_id`, y el duplicado fallando cerrado en `build()`.
- `production-composition-hardening`: la puerta de Production se extiende al progreso durable de
  read-side como cuarta capability gobernada, usando el mismo validador y la misma forma de
  error.

Si la fase de spec encuentra que un requisito existente ya implica alguno de estos, se pliega en
lugar de fabricar un delta.

## Enfoque

Reutilizar la forma probada de PROD-013 de punta a punta en vez de inventar un segundo
mecanismo: los dos SPIs declaran durabilidad con `is_durable()`, la raíz de composición sostiene
el par construido por el host, y un validador llama al ya existente
`require_durably_configured(profile, durably_configured, capability, fix)` con
`durably_configured` computado desde el `is_durable()` de ambos stores — nunca desde
`.is_some()`, que el propio doc comment de esa función prohíbe explícitamente.

El rechazo viaja por un camino que ya existe y no requiere plumbing nuevo:
`AppBuilder::build()` → `RuntimeBuilder::try_build()` (`builder.rs:1146`) →
`validate_persistence_profile()` (`:777`) → `RuntimeError` →
`CompositionError::Validation(#[from] RuntimeError)`.

La condicionalidad de la puerta refleja exactamente la del effect store.
`validate_persistence_profile` ya devuelve `Ok(())` temprano cuando `effect_executors.is_empty()`,
porque sin executor registrado no se construye ningún effect store y no hay nada volátil que
rechazar. El equivalente del read-side es "ninguna proyección registrada": IS-5 se desprende del
mismo razonamiento, no es un caso especial.

Lo genuinamente nuevo es solo el sujeto del chequeo. El registro usa como clave `projection_id`
(D-3), que no es un concepto nuevo — ya es la identidad que `ProjectionSpec` lleva y ya es el
componente inicial de ambas tuplas de clave, offset y dedup. El framework registra, clasifica y
valida; nunca construye (D-5).

## Semántica Requerida

```
Dado una composición que declara Profile::Production
Cuando no hay ningún progreso de proyección de read-side registrado
Entonces build() tiene éxito — una aplicación solo de comandos o sin read-side nunca
     es forzada a registrar un store dummy.

Dado una composición que declara Profile::Production
Cuando el OffsetStore y el DedupStore de una proyección están ambos registrados y ambos
     son durables
Entonces build() tiene éxito.

Dado una composición que declara Profile::Production
Cuando cualquiera de los dos stores del par de una proyección registrada es volátil
Entonces build() es rechazado en tiempo de composición/bootstrap — nunca diferido a
     ProjectionSpec::new(), TagSchedulerImpl::spawn() ni al primer batch — con un error
     que nombra la capability faltante/no durable y la llamada exacta que lo corrige.

Dado una composición que declara Profile::Dev (el default)
Cuando se usan offset y dedup stores en memoria/volátiles
Entonces el comportamiento no cambia, byte por byte respecto de hoy.

Dado el par de progreso de una proyección registrado dos veces para el mismo projection_id
Cuando se llama build()
Entonces la construcción falla con un error de composición que identifica el duplicado, y
     el primer registro es el que habría resuelto si la construcción hubiera tenido éxito.
```

## Áreas Afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/domain/src/read_side/offset.rs`, `dedup.rs` | Modificado | Método por defecto `is_durable(&self) -> bool { false }` en cada SPI (IS-1) |
| `crates/service-sdk/src/app/mod.rs` (`AppBuilder`, `build()` en :811) | Modificado | Superficie de registro con clave `projection_id`, dup-guarded vía el latch `pending_error` existente (IS-2, IS-3) |
| `crates/service-sdk/src/runtime/builder.rs` (`validate_persistence_profile` en :777, `try_build` en :1146) | Modificado | Rama de read-side agregada al único validador existente — nunca un segundo chequeo paralelo (IS-4, IS-5) |
| `crates/persistent-entity/src/profile.rs` | Modificado (solo doc) | `require_durably_configured` reutilizado verbatim, firma sin cambios; solo se corrige el doc comment obsoleto de `Profile::Production` (IS-10, D-8) |
| `examples/reference-app/src/read_side/mod.rs` (`ReadSideHandles::new` en :103) | Modificado | El camino de Production ya no hardcodea `InMemoryOffsetStore` / `InMemoryDedupStore` (IS-7) |
| `examples/reference-app/src/read_side/store.rs` | Modificado | Par durable falso solo de test agregado; el par en memoria se conserva para Dev/tests (IS-8, OOS-9) |
| `crates/runtime/src/read_side/scheduler.rs` (`ProjectionSpec` :175, `spawn` :276) | Intocado | Sin `Profile`, sin cambio de firma (OOS-3) |
| `crates/domain/src/read_side/{session,runner}.rs` | Intocado | Sin cambio semántico de polling/dedup/offset (OOS-3) |
| `crates/domain/src/read_side/projection_state_store.rs`, `ReadSideProcessor` | Intocado | OOS-1 / D-4 |
| `crates/persistence/src/postgres/` | Intocado | No se construye ningún backend durable de read-side (OOS-2) |
| `openspec/specs/{read-side,application-composition,production-composition-hardening}/spec.md` | Modificado | Deltas según IS-9 |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| R-1 | El camino directo `ProjectionSpec::new` + `spawn` queda sin gobierno: un host que nunca registre todavía puede correr una proyección volátil bajo Production (OOS-7) | Alta | Aceptado por diseño (D-2 rechaza el único mecanismo que lo cierra). IS-7 hace que el camino de Production del host de referencia pase por la composición, con lo que la referencia sigue siendo un ejemplo vivo y no un contraejemplo. Es la misma clase residual que R-1 de PROD-013 ("hay que acordarse del profile"), no una nueva |
| R-2 | El registro podría volverse decorativo: un host registra un par durable en la raíz de composición y le entrega un par *distinto* y volátil a `ProjectionSpec` | Media | El invariante de IS-2 es que el registro *es* el par de progreso de la proyección, no una declaración paralela sobre él. `design.md` DEBE indicar cómo el par registrado llega a `ProjectionSpec` — el recableado de referencia de IS-7 es la prueba de que llega |
| R-3 | No existe ningún backend durable de read-side en el árbol, así que un host de Production que adopte esto debe aportar su propia implementación o ser rechazado | Alta | Nombrado explícitamente (OOS-2, F-1). Un rechazo es estrictamente mejor que la volatilidad silenciosa de hoy, y F-1 es el sucesor nombrado. `design.md` no debe ablandar la puerta para compensar |
| R-4 | El registro por proyección diseña multiplicidad contra una realidad de una sola proyección (reference-app corre una) | Media | La evidencia de D-3: la clave es `projection_id`, que ya existe en ambas tuplas de clave de los stores y en `ProjectionSpec`; no se inventa ningún concepto nuevo, y N=1 es el caso degenerado. Un slot global sería la elección más riesgosa — prohíbe estructuralmente lo que el `D`/`O` por instancia de `ProjectionSpec` ya permite |
| R-5 | El delta de spec de `read-side` se lee como una reversión del non-goal de CORE-026 | Media | D-5 fija la redacción sobre dos ejes nombrados. `sdd-spec` DEBE enunciar ambos — "el framework construye/define por defecto" sin afectar, "la raíz de composición acepta y valida" nuevo — no uno solo |
| R-6 | Presupuesto de revisión: el recableado de reference-app más tres deltas de spec más la puerta plausiblemente supera las 400 líneas cambiadas | Media | `sdd-tasks` lo pronostica. Primer slice natural: IS-1 + IS-2 + IS-3 + IS-4 + IS-5 + IS-8 (framework y sus tests), con IS-7 + IS-9 + IS-10 como segundo |
| R-7 | Que `AppBuilder` gane su primera conciencia de read-side invita a scope creep hacia que `App` sea dueño del ciclo de vida del read-side | Media | OOS-3 / OOS-7: la raíz de composición registra y valida; nunca levanta, posee ni detiene el poll loop. `ReadSideHandles::spawn()` sigue fuera de `App` |
| R-8 | PROD-013 sigue sin archivar en `develop`, así que este cambio se apoya en un predecesor en vuelo | Baja | Verificado como no bloqueante durante la exploración: `cargo test --workspace` y `cargo clippy --workspace -- -D warnings` ambos salen con 0 en el verify report de PROD-013; sus casillas sin marcar son administrativas. `require_durably_configured` está en `develop` y se reutiliza sin modificar |

## Seguimientos Nombrados (deliberadamente no absorbidos)

- **F-1** — Un store durable de progreso de read-side en Postgres (`PostgreSQLOffsetStore` +
  `PostgreSQLDedupStore`), con esquema, migración y cobertura de conformidad. Reservado como un
  futuro PROD-014B **o equivalente** — el identificador intencionalmente no se compromete en
  firme aquí. El propio plan archivado de CORE-005 ya listaba esos archivos; nunca se
  construyeron.
- **F-2** — Un `ReadSideStore` durable (la vista de eventos que la proyección consulta), si
  alguna vez se quiere una vista de eventos de read-side durable (D-7).
- **F-3** — Eliminar o reubicar el fragmento abandonado `ProjectionStateStore` /
  `ReadSideProcessor` (D-4).
- **F-4** — Cerrar R-1 de forma estructural, si alguna vez se juzga que vale el costo de capas
  que el arquitecto rechazó en D-2.

## Plan de Rollback

Aditivo e inerte por defecto. `Profile::Dev` es el default y reproduce exactamente el
comportamiento de hoy, así que revertir es: quitar los dos métodos por defecto `is_durable()`, la
superficie de registro y su dup-guard, la rama de read-side dentro de
`validate_persistence_profile()`, el par durable falso solo de test, y restaurar la construcción
en `ReadSideHandles::new()`. No se toca ningún esquema, migración, dato, formato de persistencia
ni comportamiento de almacenamiento en runtime — este cambio escribe únicamente validación y
registro. Los deltas de spec revierten en un commit. Todo call site existente queda intocado
tanto por el cambio como por la reversión.

## Dependencias

- PROD-013 (`Profile`, `require_durably_configured`,
  `PersistenceCompositionError::NotConfigured`, `CompositionError::Validation`) — reutilizado
  verbatim, no modificado.
- La superficie `ProjectionSpec` / `TagSchedulerImpl::spawn` de CORE-026 — consumida sin cambios.
- El cableado de read-side existente de `examples/reference-app` — recableado, no reconstruido.
- Ninguna dependencia externa, crate, servicio, backend ni infraestructura nueva.

## Criterios de Éxito

- [ ] **SC-1** — `Profile::Production` sin read-side registrado construye exitosamente.
- [ ] **SC-2** — `Profile::Production` con un par registrado cuyo `OffsetStore` y `DedupStore`
      son ambos durables construye exitosamente.
- [ ] **SC-3** — `Profile::Production` con cualquiera de los dos stores de un par registrado en
      estado volátil es rechazado en `AppBuilder::build()`, y el rechazo es observable allí — no
      en `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()` ni en el primer batch.
- [ ] **SC-4** — Ese rechazo nombra tanto la capability faltante/no durable como la llamada
      exacta que lo corrige, con la forma de
      `PersistenceCompositionError::NotConfigured { capability, fix }`.
- [ ] **SC-5** — `Profile::Dev` con stores volátiles no cambia; `cargo test --workspace` muestra
      cero fallas nuevas y ningún call site existente requirió modificación.
- [ ] **SC-6** — Registrar el mismo `projection_id` dos veces falla cerrado en `build()`, y el
      primer registro es el que habría resuelto.
- [ ] **SC-7** — Registrar solo un store del par no es representable a través de la superficie
      pública — una configuración parcial no puede pasar la validación como si ambos estuvieran
      cubiertos.
- [ ] **SC-8** — La durabilidad se determina exclusivamente por `is_durable()` alimentando a
      `require_durably_configured`. Ningún `TypeId`, downcast, coincidencia por nombre de tipo u
      otra heurística aparece en el cambio, y la firma de `require_durably_configured` queda sin
      modificar.
- [ ] **SC-9** — `ProjectionSpec`, `TagSchedulerImpl`, `ReadSideSession` y `ReadSideRunner` no
      contienen ninguna referencia a `Profile`, y su semántica de polling/dedup/offset/orden no
      cambia.
- [ ] **SC-10** — `ReadSideHandles::new()` ya no construye `InMemoryOffsetStore` /
      `InMemoryDedupStore` en el camino de composición de Production, y el par de read-side de
      Production del host de referencia se origina en la raíz de composición.
- [ ] **SC-11** — El delta de `read-side` enuncia ambos ejes explícitamente: la construcción de
      stores por parte del framework sigue fuera de alcance y sin afectar; la aceptación y
      validación en la raíz de composición de un par construido por el host es nueva. No se lee
      como una reversión de CORE-026.
- [ ] **SC-12** — `ProjectionStateStore` y `ReadSideProcessor` no aparecen en ninguna parte del
      cambio entregado, y no se construye ningún backend durable real de read-side.
- [ ] **SC-13** — El doc comment de `Profile::Production` ya no afirma que el read-side no tiene
      slot en la raíz de composición, y nombra el alcance sucesor correcto.
