# Especificación: Endurecimiento de la Composición de Producción

> Documento acompañante — identificadores 1:1 con `spec.md` (fuente de
> verdad canónica). Incorpora la spec base de PROD-013 y el delta de
> PROD-014A (progreso durable del read-side como cuarta capacidad
> gobernada).

## Propósito

Una composición declarada como producción NUNCA debe arrancar sobre
almacenamiento volátil porque una capacidad persistente durable no fue
cableada explícitamente. Esta spec define `Profile::Production` como una
puerta de opt-in explícita que rechaza el arranque — con un error
accionable — cuando cualquiera de las cuatro capacidades persistentes
observables desde la raíz de composición (event store, snapshot store,
effect store, progreso durable del read-side) carece de una
implementación durable configurada explícitamente. `Profile::Dev` (el
valor por defecto) preserva el comportamiento actual byte a byte.

## Requisitos

### Requisito: Declaración Explícita de Profile en la Raíz de Composición

El sistema DEBE proveer un enum `Profile` con exactamente dos variantes,
`Profile::Dev` y `Profile::Production`, y un método del builder en la raíz
de composición para fijarlo. `Profile::Dev` DEBE ser el valor por defecto
cuando no se declara ningún profile.

#### Escenario: Sin profile declarado se preserva el default actual
- DADO una composición que nunca llama al método del builder que fija el
  profile
- CUANDO la composición se construye
- ENTONCES se comporta como `Profile::Dev`, idéntico al comportamiento
  actual

#### Escenario: La declaración explícita fija Production
- DADO una composición que llama al método del builder con
  `Profile::Production`
- CUANDO la composición se construye
- ENTONCES se evalúa bajo las reglas de `Profile::Production`

### Requisito: Puerta del Event Store Bajo Production

Bajo `Profile::Production`, `EntityRuntimeBuilder::build()` DEBE rechazar el
arranque cuando no se configuró explícitamente un event store durable, con
un error que nombra la capacidad de event store y
`EntityRuntimeBuilder::with_event_store()`.

#### Escenario: Event store faltante rechazado bajo Production
- DADO `Profile::Production` y ninguna llamada a `.with_event_store()`
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES es rechazado con un error que nombra el event store y
  `EntityRuntimeBuilder::with_event_store()`
- Y `InMemoryEventStore` nunca se construye

### Requisito: Puerta del Snapshot Store Bajo Production

Bajo `Profile::Production`, `EntityRuntimeBuilder::build()` DEBE rechazar el
arranque cuando no se configuró explícitamente un snapshot store durable,
con un error que nombra la capacidad de snapshot store y
`EntityRuntimeBuilder::with_snapshot_store()`.

#### Escenario: Snapshot store faltante rechazado bajo Production
- DADO `Profile::Production` y ninguna llamada a `.with_snapshot_store()`
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES es rechazado con un error que nombra el snapshot store y
  `EntityRuntimeBuilder::with_snapshot_store()`
- Y `InMemorySnapshotStore` nunca se construye

### Requisito: Puerta del Effect Store Bajo Production, Exigida en el Arranque, Condicionada a un Executor Registrado

Bajo `Profile::Production`, cuando hay al menos un effect executor
registrado, la composición DEBE rechazar el arranque cuando no se
configuró un effect store, nombrando la llamada de configuración de la
superficie de composición en uso (`RuntimeBuilder::with_effect_store()` o
`AppBuilder::effect_store()`). Esto cierra el mismo defecto de volatilidad
silenciosa que cierran las puertas de event store y snapshot store: el
effect store tiene un fallback real hacia `InMemoryEffectStore`
(`crates/service-sdk/src/runtime/builder.rs:811`) que corre en el arranque,
no una falla diferida al primer uso. Cuando no hay ningún effect executor
registrado, no se construye ningún effect store, así que no hay nada
volátil que custodiar.

#### Escenario: Effect store faltante rechazado en el arranque cuando hay un executor registrado
- DADO `Profile::Production`, al menos un effect executor registrado, y
  ninguna llamada a `.with_effect_store()` / `.effect_store()`
- CUANDO la composición construye
- ENTONCES es rechazada inmediatamente, nombrando la capacidad faltante y
  la llamada que la corrige — el mismo fallback silencioso en el arranque
  que cierran las puertas de event y snapshot store, nunca dejado a fallar
  en el primer uso

#### Escenario: Ningún effect executor registrado significa nada que custodiar
- DADO `Profile::Production` y ningún effect executor registrado
- CUANDO la composición construye
- ENTONCES tiene éxito sin importar si se configuró un effect store — no se
  construye ningún effect store, así que nada volátil es alcanzable

### Requisito: La Configuración Parcial de Event/Snapshot Bajo Production Ya Está Cubierta Por las Puertas de Cada Capacidad

Bajo `Profile::Production`, si exactamente uno de
`{event_store, snapshot_store}` está configurado y el otro no,
`EntityRuntimeBuilder::build()` DEBE rechazar el arranque — no mediante un
chequeo de configuración parcial separado, sino porque la puerta propia de
la capacidad faltante (Puerta del Event Store / Puerta del Snapshot Store,
arriba) ya lo rechaza: exactamente uno faltante sigue siendo uno faltante.
Bajo `Profile::Dev`, la configuración parcial permanece válida, sin
cambios respecto del comportamiento actual — incluyendo los 15 sitios de
llamada existentes que configuran exactamente uno de los dos stores hoy
(`design.md` §Evidence Corrections, EC-1), entre ellos la propia raíz de
composición de producción de la reference app, `observed_entity_runtime`
(`lib.rs:502`).

#### Escenario: Configuración parcial rechazada bajo Production vía su propia puerta de capacidad
- DADO `Profile::Production` con `.with_event_store()` llamado y
  `.with_snapshot_store()` nunca llamado
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES es rechazado por la Puerta del Snapshot Store (nombrando el
  snapshot store y `.with_snapshot_store()`), no por un chequeo de
  configuración parcial separado

#### Escenario: La configuración parcial permanece válida bajo Dev, sin cambios
- DADO `Profile::Dev` (el default, sin profile declarado) con
  `.with_event_store()` llamado y `.with_snapshot_store()` nunca llamado
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES tiene éxito, cayendo a `InMemorySnapshotStore` para la
  capacidad sin configurar, byte a byte como antes de este cambio

### Requisito: Puerta de Progreso Durable del Read-Side Bajo Production, Aplicada en el Bootstrap, Condicionada a una Proyección Registrada

Bajo `Profile::Production`, cuando al menos el par de progreso durable de
una proyección (`OffsetStore` + `DedupStore`) está registrado en la raíz
de composición, `AppBuilder::build()` DEBE rechazar el bootstrap cuando
cualquiera de los dos stores de ese par no sea durable, nombrando la
capacidad faltante o no durable y la llamada de registro exacta que la
resuelve. Esto refleja la condicionalidad de la puerta del effect store:
cuando no hay ninguna proyección registrada, no existe ningún par de
progreso durable del read-side que construir, así que no hay nada
volátil que rechazar — una aplicación solo de comandos o sin read-side
nunca se ve forzada a registrar un store ficticio. La puerta recorre el
mismo camino existente que estableció PROD-013
(`AppBuilder::build()` -> `RuntimeBuilder::try_build()` ->
`validate_persistence_profile()` -> `RuntimeError` ->
`CompositionError::Validation`), nunca un segundo validador paralelo. La
durabilidad se determina exclusivamente por `is_durable()` en cada store,
alimentando `require_durably_configured()` — nunca por `.is_some()` ni
por ninguna otra heurística.

#### Escenario: Un store volátil en un par registrado es rechazado en el bootstrap

- DADO `Profile::Production` y una proyección registrada cuyo
  `OffsetStore` o `DedupStore` no es durable
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES se rechaza, nombrando la capacidad faltante o no durable y la
  llamada de registro exacta que la resuelve — nunca diferido a
  `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, ni el primer lote

#### Escenario: Ninguna proyección registrada implica que no hay nada que rechazar

- DADO `Profile::Production` y ningún progreso de proyección de read-side
  registrado
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES tiene éxito — no se construye ningún par de progreso durable
  del read-side, así que no hay nada volátil alcanzable

#### Escenario: Ambos stores durables tiene éxito

- DADO `Profile::Production` y una proyección registrada cuyo
  `OffsetStore` y `DedupStore` son ambos durables
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES tiene éxito

#### Escenario: El profile Dev con stores volátiles no cambia

- DADO `Profile::Dev` (por defecto) y una proyección registrada usando
  `OffsetStore`/`DedupStore` en memoria/volátiles
- CUANDO se ejecuta `AppBuilder::build()`
- ENTONCES tiene éxito, byte por byte como antes de este cambio

### Requisito: El Comentario de Documentación de Profile::Production Refleja el Slot de Progreso Durable del Read-Side

El comentario de documentación de `Profile::Production`
(`crates/persistent-entity/src/profile.rs`) NO DEBE afirmar que la
persistencia de read-side/proyección "has no such slot yet and is
deliberately not governed here" una vez que este cambio se despliegue —
esa afirmación se vuelve falsa en el momento en que el registro existe.
El comentario DEBE en cambio nombrar el par de progreso durable del
read-side como una cuarta capacidad gobernada junto al event store, el
snapshot store y el effect store, y NO DEBE apuntar a un identificador
sucesor como trabajo aún pendiente para una capacidad que este cambio ya
gobierna.

#### Escenario: El comentario de documentación lista la cuarta capacidad gobernada

- DADO el comentario de documentación de `Profile::Production` después de
  que este cambio se despliega
- CUANDO se lee
- ENTONCES lista el par de progreso durable del read-side junto al event
  store, el snapshot store y el effect store como capacidades
  gobernadas, y no afirma que el read-side carece de un slot en la raíz
  de composición

### Requisito: Un Único Predicado Compartido Es la Única Fuente de Verdad de la Regla

Exactamente un predicado compartido DEBE decidir "production declarado +
capacidad no configurada de forma durable = rechazar" para las cuatro
capacidades (event store, snapshot store, effect store, progreso durable
del read-side). Debido a que las capacidades viven a través de un límite
de crate unidireccional (`persistent-entity` no puede ver los tipos de
effect-store ni de read-side de `service-sdk`), este predicado no puede
inspeccionar ninguno de los builders directamente: cada superficie de
composición (`validate_persistence()` de `EntityRuntimeBuilder`,
`validate_persistence_profile()` de `RuntimeBuilder`, incluyendo su rama
de read-side) DEBE calcular localmente su propia respuesta para su
capacidad y pasarla al único predicado compartido — nunca reafirmar la
decisión de rechazar/permitir por su cuenta. NO DEBE existir una segunda
definición, mantenida de forma independiente, de la decisión en ningún
punto del camino de composición.

(Anteriormente: alcanzaba tres capacidades, event store/snapshot
store/effect store; ahora incluye el progreso durable del read-side como
una cuarta, validada por el mismo `validate_persistence_profile()` que ya
usa el effect store.)

#### Escenario: La decisión de las tres capacidades originales pasa por el mismo predicado

- DADO el camino de composición desde `EntityRuntimeBuilder` y
  `RuntimeBuilder`/`AppBuilder`
- CUANDO se inspecciona el código base en busca de lógica de puerta de
  capacidad para event store, snapshot store o effect store
- ENTONCES cada punto de la puerta calcula su propia respuesta local y la
  pasa al único predicado compartido que decide rechazar-o-permitir;
  ningún punto reimplementa esa decisión por su cuenta

#### Escenario: La decisión de la cuarta capacidad pasa por el mismo predicado

- DADO la puerta de progreso durable del read-side agregada por este
  cambio
- CUANDO se inspecciona el código base en busca de su lógica de puerta
- ENTONCES calcula su propia respuesta local (¿son durables ambos stores
  de un par registrado?) y la pasa al mismo predicado compartido que ya
  usan las otras tres capacidades — no existe una decisión separada,
  mantenida de forma independiente, exclusiva del read-side

### Requisito: Los Rechazos Son Accionables

Todo rechazo bajo este spec DEBE nombrar tanto la capacidad faltante como
la llamada de configuración exacta que la resuelve.

(Anteriormente: enumeraba tres capacidades en su escenario; ahora incluye
el progreso durable del read-side como una cuarta.)

#### Escenario: El error nombra la capacidad y la solución

- DADO cualquier rechazo producido por la puerta de este spec
- CUANDO se inspecciona el error
- ENTONCES nombra la capacidad faltante (event store, snapshot store,
  effect store, o progreso durable del read-side) y la llamada de
  registro o de builder exacta que la configura

### Requisito: Las Composiciones Sin Production Compilan y Pasan Sin Modificación

Toda composición que no declara `Profile::Production` DEBE seguir
compilando y pasando sin modificación, incluyendo los 67 sitios de llamada
existentes a `EntityRuntimeBuilder::new()`. `cargo test --workspace` DEBE
mostrar cero fallas nuevas atribuibles a este cambio.

#### Escenario: Un sitio de llamada sin modificar sigue construyendo sobre almacenamiento en memoria
- DADO un sitio de llamada existente que nunca declara
  `Profile::Production` y nunca configura event/snapshot/effect store
- CUANDO se reconstruye después de que este cambio se envía
- ENTONCES compila y `EntityRuntimeBuilder::build()` sigue teniendo éxito
  sobre almacenamiento en memoria, byte a byte como antes de este cambio

#### Escenario: La suite de tests completa no muestra fallas nuevas
- DADO la suite de tests completa del workspace antes y después de este
  cambio
- CUANDO corre `cargo test --workspace` después del cambio
- ENTONCES reporta cero fallas nuevas causadas por este cambio

### Requisito: La Regla de Completitud de Persistencia Está Documentada

La documentación de arquitectura DEBE establecer la regla de completitud de
persistencia: una base de datos no se considera soportada por `ego-rs`
hasta que implementa cada capacidad persistente que una composición de
producción declara usar; las capacidades faltantes NO DEBEN completarse
cayendo a almacenamiento en memoria; el soporte de backend es todo-o-nada a
través de las capacidades durables que una composición habilita. Esto es
guía prospectiva, no un reporte de que el único backend actual
(PostgreSQL) sea incumplidor.

#### Escenario: La regla se documenta como prospectiva, no como reporte de violación
- DADO la documentación de arquitectura agregada por este cambio
- CUANDO se lee
- ENTONCES establece la regla de completitud explícitamente y no señala a
  PostgreSQL, el único backend que existe hoy, como violador de ella

### Requisito: El Límite con PROD-005 Está Documentado

La documentación DEBE establecer explícitamente que esta spec rechaza el
arranque mismo, antes de que nada inicie, mientras que PROD-005 (Salud,
Disponibilidad y Arranque) señala la salud de una aplicación que ya
arrancó, con modo degradado permitido para dependencias opcionales — para
que ambas nunca se confundan.

#### Escenario: El límite es legible e inequívoco
- DADO la documentación agregada por este cambio
- CUANDO un lector compara el alcance de esta spec con el de PROD-005
- ENTONCES el texto establece llanamente que esta spec decide si la app
  puede arrancar, y PROD-005 describe una app que ya lo hizo

### Requisito: La Reference App Declara su Profile a Través de EntityEventStores

`build_runtime_with` (`lib.rs:567`) es el punto de entrada compartido tanto
para composiciones durables como en memoria y NO DEBE hardcodear un profile
por sí mismo — hoy se llama con stores en memoria desde cuatro lugares, y
hardcodear `Profile::Production` adentro rompería cada uno de ellos. En su
lugar, `EntityEventStores` — el tipo que ya existe para que la elección del
store de respaldo esté declarada, nunca por defecto — DEBE cargar el
profile: `EntityEventStores::open(pool)` DEBE producir
`Profile::Production`, y `EntityEventStores::in_memory()` DEBE producir
`Profile::Dev`, a través de un campo privado fijado solo por esos dos
constructores. `main.rs`, que ya llama a `EntityEventStores::open()`, se
convierte en una composición `Profile::Production` sin ninguna declaración
separada que se pueda olvidar.

#### Escenario: EntityEventStores::open produce Production
- DADO `EntityEventStores::open(pool)`
- CUANDO se inspecciona el profile del valor resultante
- ENTONCES reporta `Profile::Production`

#### Escenario: EntityEventStores::in_memory produce Dev
- DADO `EntityEventStores::in_memory()`
- CUANDO se inspecciona el profile del valor resultante
- ENTONCES reporta `Profile::Dev`

#### Escenario: Los llamadores orientados a dev permanecen en Profile::Dev sin editar
- DADO `build_runtime_in_memory` (`lib.rs:311`) y
  `build_runtime_observed_in_memory` (`lib.rs:522`)
- CUANDO se inspecciona su código de composición
- ENTONCES cada una fluye a través de `EntityEventStores::in_memory()` y
  permanece en `Profile::Dev` sin ninguna modificación

### Requisito: El Snapshot Store de Producción de la Reference App Es Durable

`EntityEventStores::open(pool)` DEBE construir sus dos snapshot stores como
instancias de `PostgreSQLSnapshotStore` (ya implementado en
`crates/persistence/src/postgres/snapshot.rs:27`) sobre el mismo pool,
reemplazando el snapshot store en memoria que el camino de composición de
producción usa silenciosamente hoy. `EntityEventStores::in_memory()` DEBE
seguir construyendo instancias de `InMemorySnapshotStore`, sin cambios.
Esto cablea un backend ya existente en un constructor ya existente; no
construye ningún almacenamiento nuevo, así que permanece dentro del
No-Objetivo de esta spec de ningún backend Postgres nuevo.

Nota de implementación para `tasks`/`apply`:
`PostgreSQLSnapshotStore::save_snapshot` llama a
`tokio::task::block_in_place` (`snapshot.rs:46-48`), que panickea sobre un
runtime de Tokio de un solo hilo. Cualquier test que pueda disparar un
snapshot real contra Postgres DEBE correr bajo
`#[tokio::test(flavor = "multi_thread")]` en lugar del default de un solo
hilo.

#### Escenario: EntityEventStores::open cablea snapshot stores durables
- DADO `EntityEventStores::open(pool)`
- CUANDO se inspeccionan los snapshot stores del valor resultante
- ENTONCES ambos están respaldados por `PostgreSQLSnapshotStore` sobre el
  mismo pool, no por `InMemorySnapshotStore`

#### Escenario: EntityEventStores::in_memory mantiene snapshot stores volátiles
- DADO `EntityEventStores::in_memory()`
- CUANDO se inspeccionan los snapshot stores del valor resultante
- ENTONCES ambos permanecen como `InMemorySnapshotStore`, sin cambios
  respecto de hoy

### Requisito: Un Chequeo de Regresión Custodia la Declaración de Referencia

Un chequeo (un lint de `xtask` o un test — el mecanismo exacto es una
decisión de `design.md`) DEBE fallar el build si
`EntityEventStores::in_memory().profile()` alguna vez deja de reportar
`Profile::Dev`, o si la composición de `main.rs` — el único llamador que
alcanza `EntityEventStores::open()` — alguna vez deja de resultar en una
composición `Profile::Production`, para que la composición de referencia
siga siendo un guardián de regresión vivo, no un ejemplo de una sola vez.
Verificar el comportamiento de los constructores de `EntityEventStores` es
suficiente: con el campo profile privado y esos dos constructores como su
única fuente, no hay ninguna declaración separada en otro lugar que pueda
divergir de ellos.

#### Escenario: Remover el cableado de producción hace fallar el chequeo
- DADO una composición que alcanza `EntityEventStores::open()` que ya no
  resulta en una composición `Profile::Production`
- CUANDO corre el chequeo de regresión
- ENTONCES el build falla, nombrando la declaración faltante

#### Escenario: La declaración presente pasa el chequeo
- DADO `EntityEventStores::open()` produciendo `Profile::Production` y
  `EntityEventStores::in_memory()` produciendo `Profile::Dev`, ambos
  cableados según lo requerido
- CUANDO corre el chequeo de regresión
- ENTONCES pasa

## No-Objetivos

- **Ningún backend Postgres nuevo.** Los event, snapshot y effect stores
  durables ya existen; esta spec valida la configuración y cablea el
  `PostgreSQLSnapshotStore` ya existente en `EntityEventStores::open` — no
  implementa ni construye almacenamiento nuevo.
- **La puerta de read-side/proyección/checkpoint ya está incluida.**
  PROD-014A — Composición de Persistencia de Read-Side & Store Durable —
  introduce un registro genérico de persistencia read-side/proyección en
  la raíz de composición y aplica la política idéntica de fail-closed que
  esta spec establece: capacidad no configurada → válida; capacidad
  configurada con un backend no durable/en memoria → arranque rechazado;
  capacidad configurada con un backend durable → válida. (Anteriormente
  diferido como no-objetivo en PROD-013; PROD-014A lo convierte en un
  requisito.)
- **Ningún tema de observabilidad, alta disponibilidad, migración u otro
  tema de endurecimiento de producción.**
- **Ningún segundo motor de base de datos.** La regla de completitud de
  persistencia es prospectiva; no hay nada contra qué validarla hoy.
- **Ninguna decisión sobre el Enfoque C** (invertir el default a
  fail-closed con un opt-out nombrado, siguiendo a
  `IdempotencyEnforcementMode`) ni su migración de ~32 sitios de llamada.
  Registrado como alternativa evaluada y diferida en `design.md`.
- **Ninguna eliminación, deprecación u ocultamiento de las implementaciones
  en memoria.** Permanecen válidas, explícitas y de primera clase para
  `Profile::Dev` y los tests.
- **Ninguna reapertura de la regla de idempotencia de PROD-012 ni de su
  modo de enforcement.**
- **Ningún patrón outbox/inbox.** Confirmado ausente del código; no
  aplicable.
