# Propuesta: PROD-014C — Reclamo Atómico de Eventos de Lado de Lectura

> Compañero de revisión en español. Fuente de verdad canónica: `proposal.md` (identificadores 1:1).

## Intención

PROD-014B entregó progreso durable de lado de lectura, pero solo bajo una restricción de adopción
**no impuesta**: escritor-único-por-`(projection_id, tag, tenant)`. Nada detecta ni rechaza una
segunda réplica. `ReadSideSession::execute()` (`crates/domain/src/read_side/session.rs:91-176`)
ejecuta `handler.handle()` entre `dedup_store.seen()` y `dedup_store.mark_seen()` sin transacción
compartida, sin bloqueo y sin lease — dos réplicas pueden observar ambas `seen() == false` e
invocar ambas el handler. `ego-rs` apunta a producción distribuida, así que su propio despliegue
objetivo documentado queda hoy fuera de su propia garantía.

PROD-014C obtiene **exclusión antes de que el handler se ejecute**: para una identidad de reclamo,
a lo sumo un trabajador mantiene un reclamo de procesamiento válido a la vez, entre procesos, con
recuperación ante caídas y rechazo del propietario obsoleto. Esto es *propiedad única y válida de
procesamiento*, nunca exactamente-una-vez.

## Decisiones Activas

| ID | Decisión | Fundamento |
|----|----------|------------|
| D-1 | **La identidad de reclamo es exactamente `(projection_id, tag, tenant)`** — no por evento, no por proyección, no global (IS-1) | Esta tripleta ya es la unidad que posee un único offset que avanza monotónicamente: la propia documentación de `OffsetStore` declara "Offsets are independent per (projection_id, tag, tenant) tuple" (`crates/persistence-api/src/read_side/offset.rs:53`) y `013_create_projection_offsets.sql:24-25` la declara como `PRIMARY KEY (projection_id, tag, tenant)`. Reclamar exactamente con esta granularidad preserva trivialmente la promesa de orden existente — `ReadSideStore::fetch` retorna "events with `event_version > offset` in ascending version order" por `(tenant, tag)` (`crates/persistence-api/src/read_side/store.rs:30`), y solo el poseedor del reclamo invoca `fetch`/`handle` para ese flujo. Reclamar por evento no serializaría nada que el orden por flujo no serialice ya, y multiplicaría los viajes de ida y vuelta por lote (R-3). Un reclamo más grueso (global, o por `projection_id`) serializaría entre sí a tenants y tags no relacionados. La identidad más estrecha del dedup, `(projection_id, tag, event_id)` (migración `014`, sin columna de tenant), deliberadamente **no** es la identidad de reclamo: el dedup rastrea presencia de eventos, el reclamo posee el progreso de un flujo con alcance de tenant |
| D-2 | **Un puerto nuevo, no un método agregado a `OffsetStore` o `DedupStore`** — como cuestión de principio. El conjunto exacto de métodos y el mecanismo de adquisición quedan abiertos para `sdd-design` (ver Enfoque) | `OffsetStore` hoy son tres métodos — `is_durable`, `read_offset`, `write_offset` (`crates/persistence-api/src/read_side/offset.rs:55-84`) — y su `write_offset` es contractualmente una sobrescritura plana con "no compare-and-swap, no expected-previous-offset check" (`crates/persistence/src/postgres/read_side_offset.rs:5-14`, PROD-014B D-3/L-5). La exclusión es una propiedad distinta del registro de posición, y el registro de dedup es una tercera. Ambos traits están entregados y archivados (PROD-014A/PROD-014B), ambos llevan impls genéricas de reenvío `Arc<T>` (`offset.rs:86+`), ambos tienen implementaciones en memoria, y ambos son leídos en vivo por la compuerta de Producción (`crates/service-sdk/src/runtime/builder.rs:879-891`) — un método requerido nuevo rompe a todo implementador existente, y uno con default entrega un default que ninguna implementación honesta puede satisfacer. Nótese que esto **no** se elige para evitar un delta de especificación: extender un contrato entregado es un delta de requisito MODIFICADO en cualquier caso (exploración §Recomendación). Se elige para que el delta caiga sobre una capacidad nueva en lugar de reabrir dos archivadas |
| D-3 | **La prueba de propiedad (fencing) se lleva como requisito de este cambio, no como endurecimiento opcional diferido a un seguimiento** (IS-3, SC-3) | La toma de control por expiración (IS-2) sin prueba de propiedad es estrictamente peor que no tener toma de control. `write_offset` es última-escritura-gana por contrato (la cita de D-2), así que un trabajador cuyo lease expiró a mitad de lote y luego reanudó seguiría siendo aceptado por `write_offset` y `mark_seen` — convirtiendo un defecto de doble ejecución en un defecto de regresión de offset, donde un propietario obsoleto retrocede una posición que un propietario vivo ya había avanzado. El workspace ya resolvió el problema análogo del lado de escritura exactamente así: `crates/persistence/src/postgres/reservation.rs` acuña un token de fencing estrictamente mayor dentro del mismo `UPDATE` condicional de toma de control que re-apropia la fila, y su ayudante compartido `mutate_owned` verifica la tupla completa `(tenant, key, owner, fencing_token, lease_until > now)` en una sola cláusula `WHERE` por sentencia — nunca una ventana separada de verificar-luego-escribir. Entregar la toma de control sin fencing entregaría un mecanismo que *se lee* como exclusión mientras un propietario obsoleto escribe a través de él |
| D-4 | **La expiración del lease se mide contra un `Clock` inyectado, nunca contra el `now()` de la base de datos** (R-5) | `PostgresOperationReservationStore` mantiene `clock: Arc<dyn Clock>` inyectado en construcción (`crates/persistence/src/postgres/reservation.rs:70-73, 85-87`) y compara la expiración contra él — AD-8 de PROD-012A, ya probado bajo contención real. Aplican sin cambios las mismas dos razones: la expiración pasa a ser determinísticamente comprobable, y el momento de la toma de control deja de depender en silencio del reloj de pared de cada réplica |
| D-5 | **`Profile::Production` falla cerrado cuando no hay mecanismo de reclamo durable registrado** (IS-6) | Es lo único que salda la Intención. La propia limitación vinculante de PROD-014B declara que el framework "is adoptable in Production only under single-writer-per-`(projection_id, tag, tenant)`" y que "nothing in this change detects or refuses" una segunda réplica (L-3). Un PROD-014C que entregara el mecanismo pero lo dejara opcional dejaría el mismo agujero de mala configuración silenciosa una capa más arriba. La forma de la compuerta ya existe y no requiere invención: `require_durably_configured` (`crates/persistent-entity/src/profile.rs:51-63`) más la rama de lado de lectura `validate_read_side_progress_profile` (`crates/service-sdk/src/runtime/builder.rs:879-891`), alcanzada mediante el registro de `AppBuilder::read_side_progress` (`crates/service-sdk/src/app/mod.rs:633-651`). PROD-014A R-3 estableció el precedente de que un rechazo es estrictamente mejor que volatilidad silenciosa. Si la compuerta reutiliza `require_durably_configured` textualmente o necesita un predicado hermano es decisión del diseño |
| D-6 | **La ubicación es `crates/persistence/src/postgres/`, continuando la secuencia plana de migraciones en `016+` — no en `015`** | PROD-014B D-5 estableció tanto la ubicación como la regla de secuencia plana (la secuencia independiente `001/002` de `crates/effect-store` se justificó como propiedad de un crate ya separado, no como regla general). La secuencia en `develop` termina hoy en `015_fix_aggregates_tenant_null_uniqueness.sql`, con `013_create_projection_offsets.sql` y `014_create_projection_dedup.sql` inmediatamente antes, así que el siguiente número libre es `016`. **Esto corrige un error del borrador previo a esta enmienda, que decía `015+`** — ese número está ocupado y habría colisionado al aplicar. Esta decisión solo obliga si el diseño elige un mecanismo con fila de reclamo; si elige un advisory lock de PG sin fila de reclamo (una opción que el Enfoque deja abierta), no se entrega migración y esta decisión queda inerte |
| D-7 | **La contención se prueba únicamente contra PostgreSQL real, en `integration-tests/`** (IS-7, SC-7) | `ego-rs-testing-strategy` prohíbe que cualquier prueba unitaria alcance una base de datos real; `integration-tests` es un workspace separado y el único lugar donde se admite infraestructura real (PROD-014B D-8). Ya viven 22 suites `*_postgres.rs` bajo `integration-tests/tests/infrastructure/` que obtienen su base de datos de `isolated_database()`; cuatro de ellas — `concurrent_replicas_postgres.rs`, `lease_contention_postgres.rs`, `fencing_window_postgres.rs`, `takeover_fencing_postgres.rs` — ya hacen competir contendientes reales contra `operation_reservations`, y `read_side_progress_postgres.rs` es la propia suite de conformidad de lado de lectura de PROD-014B. Hay además una razón específica de este cambio: una afirmación de atomicidad es una afirmación sobre lo que la base de datos hace bajo contención real. Una simulación a nivel unitario afirma el comportamiento de un doble de prueba, así que puede pasar mientras el mecanismo real está roto |
| D-8 | **La frontera es la cantidad de ejecuciones del handler, nunca la cantidad de efectos externos — y el cambio entregado no debe redactarse jamás como exactamente-una-vez** (OOS-2, R-1, SC-6) | `Handler<E>` (`crates/domain/src/read_side/handler.rs`) no impone restricción alguna sobre lo que hace un handler; su documentación solo dice que las implementaciones "should be idempotent where possible", sin que nada lo imponga. Así que un reclamo perfecto acota cuántas veces el framework *invoca* el handler y no puede decir nada sobre un efecto que el handler ya realizó — la misma advertencia ya documentada para el mecanismo del lado de escritura en `crates/persistence/src/postgres/reservation.rs:22-26`. De forma independiente, la sola recuperación ante caídas ya fuerza al-menos-una-vez con una réplica y cero concurrencia: morir durante el handler, o después de que tenga éxito pero antes de `mark_seen`, re-invoca el handler al reanudar sin que intervenga ningún segundo trabajador. Por lo tanto exactamente-una-vez no está meramente fuera de alcance aquí — es inalcanzable en esta capa. Eso convierte la disciplina de nomenclatura en criterio de aceptación, con el mismo peso que PROD-014B le dio a L-1 |
| D-9 | **Sin consenso distribuido, elección de líder ni broker — y sin más backend que PostgreSQL** (OOS-1, OOS-6) | El problema es exclusión sobre una identidad de fila, que una sola fila de PostgreSQL ya serializa; `reservation.rs` prueba la forma completa de lease más fencing sin protocolo de consenso, sin elección de líder y sin broker. Introducir uno resolvería un problema materialmente más difícil que el que nombra la Intención, y agregaría superficie operativa que el framework no tiene de otro modo. Sobre backends: una auditoría completa del código bajo PROD-014B (OOS-4) encontró cero código productivo para cualquier almacén no-PostgreSQL, solo prosa ilustrativa en OpenSpec archivado. Un segundo backend es el cambio propio de un segundo adaptador, condicionado al puerto que define este |
| D-10 | **Los reintentos/backoff para errores `Transient` quedan excluidos** (OOS-3) | Real, preexistente e independiente. `crates/domain/src/read_side/error.rs:9` documenta "Retry batch with exponential backoff (max 3 retries, 100ms base, 10s max)"; un grep de `backoff`/`retry` en `crates/domain/src/read_side/` y `crates/runtime/src/read_side/` devuelve cero coincidencias fuera de ese único comentario — el callback `on_error` del scheduler solo registra en log, y el bucle espera al siguiente tick de sondeo. Las dos preocupaciones son ortogonales en ambas direcciones: el reintento decide *cuándo* se re-intenta un lote, el reclamo decide *quién* puede intentarlo, y cualquiera se entrega sin el otro |
| D-11 | **La atomicidad entre tablas para las escrituras de dedup y de offset queda excluida** (OOS-4) | Ya está ausente hoy, antes de que exista concurrencia alguna. `mark_seen` y `write_offset` viven en dos structs separados contra dos tablas separadas, cada uno ejecutando su propio `.execute(&self.pool)` con auto-commit; `crates/domain` es genérico sobre los dos puertos sin ningún manejador de transacción atravesándolo, por diseño hexagonal. El reclamo ni crea ni cierra esto: una caída a mitad del bucle de `mark_seen` antes de `write_offset` re-invoca el handler al reanudar con un lote más pequeño, y eso ocurre con un solo trabajador sosteniendo un reclamo válido todo el tiempo. Cerrarlo requeriría una transacción compartida entre dos puertos — rediseñar los contratos de PROD-014B que este cambio consume sin modificar (D-2) |
| D-12 | **La concurrencia intra-proceso entre tags queda excluida** (OOS-5) | Hoy no hay ninguna que proteger. `TagSchedulerImpl::start_projection` espera la sesión de cada tag secuencialmente en un bucle for, y `Backpressure::acquire()` se espera en línea dentro de `execute_session` en vez de regular tareas lanzadas — así que la redacción "respecting concurrency limits" del scheduler describe hoy limitación de carga, no paralelismo. El reclamo hace segura la concurrencia entre *procesos*; explotar el paralelismo entre tags dentro de un proceso es un cambio de scheduler con sus propias obligaciones de orden y contrapresión. Se excluye para que los criterios de aceptación de este cambio sigan siendo sobre exclusión y no sobre rendimiento |

## Compuerta de Atomicidad

**Se ejecutó, y recortó el alcance cuatro veces.** Los reintentos/backoff para errores `Transient`
se consideraron y se retiraron (D-10): son entregables de forma independiente en cualquier orden y
responden a una pregunta distinta. La atomicidad entre tablas de dedup y offset se consideró y se
retiró (D-11): cerrarla implica una transacción compartida entre dos puertos archivados, lo que
convertiría "obtener exclusión" en "rediseñar los contratos de persistencia de lado de lectura". La
concurrencia intra-proceso entre tags se consideró y se retiró (D-12): es un cambio de rendimiento
del scheduler, y plegarlo aquí permitiría que una regresión de rendimiento hiciera fallar una
propuesta de exclusión. Un segundo backend no-PostgreSQL se consideró y se retiró (D-9): es un
segundo adaptador contra el puerto que este cambio define.

Lo que queda es una capacidad indivisible, porque ningún elemento en alcance es entregable de forma
independiente con valor:

- **IS-1** es la decisión de identidad sobre la que se apoya todo lo demás — una decisión, no un
  entregable.
- **IS-2** por sí solo es un puerto que nadie invoca: código muerto, y peor que ausente, porque la
  existencia del trait se lee como una garantía.
- **IS-5** por sí solo es una tabla y un adaptador a través de los cuales nada reclama.
- **IS-4** por sí solo no tiene nada que adquirir — no puede existir sin IS-2 e IS-5.
- **IS-3** no puede diferirse sin entregar una garantía falsa. La toma de control sin prueba de
  propiedad permite que un propietario obsoleto escriba offsets a través de un mecanismo que se lee
  como exclusión (D-3), así que la versión de este cambio sin IS-3 no es una garantía más pequeña —
  es una equivocada.
- **IS-6** es lo que vuelve no-opcionales a IS-2..IS-5. Sin él el mecanismo existe y una composición
  de producción todavía puede ejecutar en silencio exactamente la configuración multi-réplica que
  L-3 de PROD-014B nombró como fuera de la garantía, lo que deja la Intención sin saldar. A la
  inversa, IS-6 sin IS-2..IS-5 es el modo de falla R-3 de PROD-014A: un rechazo sin nada en el árbol
  que lo satisfaga.
- **IS-7** es la única forma en que SC-1, SC-2 o SC-3 son observables en absoluto (D-7).
- **IS-8** no es adorno documental. El rustdoc de los propios adaptadores entregados,
  `ARCHITECTURE.md:211-219` y `examples/reference-app/src/read_side/mod.rs:118-126` nombran hoy a
  PROD-014C como el que cierra la brecha y declaran la restricción de escritor único como no
  impuesta. Aterrizar la imposición sin IS-8 deja la documentación entregada afirmando algo falso.

Todos los elementos nombran el mismo mecanismo — un reclamo por `(projection_id, tag, tenant)`,
adquirido antes de `fetch`, sostenido durante el lote hasta `write_offset`, probado por fencing — y
el mismo criterio de aceptación.

El pronóstico de PRs apiladas de R-4 no es un contraargumento a esta compuerta. La atomicidad
gobierna si esto es una sola capacidad; el rebanado gobierna cuántos diffs revisables la entregan.
PROD-014A llevó el mismo emparejamiento (su R-6 junto a su propio PASS), y allí las rebanadas eran
unidades de entrega de una capacidad, no propuestas separadas.

**ATOMICITY: PASS**

## Alcance

### En Alcance

- **IS-1** — Identidad de reclamo `(projection_id, tag, tenant)` — la PK de `projection_offsets` y
  la identidad documentada del propio `OffsetStore`; reclamar por flujo preserva trivialmente el
  orden por `(tenant, tag)` de `ReadSideStore::fetch`.
- **IS-2** — Un puerto de reclamo atómico: adquirir-o-rechazar, renovación de lease, liberación y
  toma de control por expiración, para que un trabajador muerto no bloquee el flujo para siempre.
- **IS-3** — Prueba de propiedad para que un trabajador que perdió su reclamo no pueda seguir
  escribiendo como propietario. El precedente `operation_reservations`
  (`crates/persistence/src/postgres/reservation.rs`) ya resuelve el problema análogo del lado de
  escritura con un token de fencing monotónico + `Clock` inyectado; PROD-014C adopta la misma forma
  salvo que el diseño pruebe que es innecesaria.
- **IS-4** — `ReadSideSession::execute()` adquiere el reclamo antes de `fetch` y lo mantiene hasta
  `write_offset`.
- **IS-5** — Un adaptador PostgreSQL durable + migración `016+` (el siguiente número libre; `015`
  está ocupado — D-6), replicando la forma de `UPDATE` condicional con CAS de `reservation.rs`.
- **IS-6** — `Profile::Production` falla cerrado cuando no hay mecanismo de reclamo durable
  registrado. Tras el cambio, el lado de lectura multi-réplica pasa a ser **SOPORTADO CON
  RESTRICCIÓN OPERATIVA EXPLÍCITA** (almacén de reclamo durable registrado; los efectos del handler
  siguen siendo al-menos-una-vez). El mecanismo de compuerta — y si replica el idiom
  `require_durably_configured` de PROD-014A — es decisión del diseño.
- **IS-7** — Pruebas de contención contra PostgreSQL real en `integration-tests/`, modeladas sobre
  `concurrent_replicas_postgres.rs` / `takeover_fencing_postgres.rs`.
- **IS-8** — Deltas de especificación según Capacidades; la documentación del adaptador y del README
  reemplaza la restricción de escritor único por la nueva restricción impuesta.

### Fuera de Alcance

- **OOS-1** — Consenso distribuido, elección global de líder, coordinador de transacciones
  distribuidas, reemplazo de consumer-groups de Kafka, rediseño del EventStore.
- **OOS-2** — Efectos secundarios **externos** exactamente-una-vez. `Handler<E>` permite I/O
  arbitrario, así que el reclamo acota únicamente la cantidad de ejecuciones del handler; la
  frontera del efecto debe portar su propio fence — la misma advertencia ya documentada en
  `reservation.rs:22-26`.
- **OOS-3** — Reintentos/backoff para errores `Transient` (documentado en
  `crates/domain/src/read_side/error.rs:9`, no implementado). Adyacente, separado.
- **OOS-4** — Atomicidad entre tablas de dedup y offset (hoy dos upserts independientes).
- **OOS-5** — Concurrencia intra-proceso por tag (`TagSchedulerImpl` es secuencial hoy).
- **OOS-6** — Cualquier backend distinto de PostgreSQL; eliminar los pares en memoria.

## Capacidades

### Capacidades Nuevas

- `read-side-event-claiming`: el contrato observable de exclusión — identidad de reclamo, rechazo
  de adquisición bajo un reclamo vivo, toma de control por expiración, rechazo del propietario
  obsoleto, y lo que aun así no promete.

### Capacidades Modificadas

- `read-side`: "Prevention of Double Handler Execution Rests on an Explicit, Unenforced
  Single-Writer Adoption Constraint" pasa a ser impuesta; "The Concurrency Gap Has a Named,
  Distinct Follow-Up" queda saldada. "Durable Dedup Bookkeeping Does Not Imply Exactly-Once
  Handler Execution" sigue siendo cierta y DEBE sobrevivir sin cambios.

`read-side-durable-progress` no necesita delta — sus no-objetivos ya asignan el reclamo aquí.

## Enfoque

Agregar un **puerto nuevo** en lugar de evolucionar `OffsetStore`/`DedupStore`, cuyos contratos ya
están entregados y archivados y cuya semántica (sobrescritura de offset; registro de dedup) es
ortogonal a la exclusión. Darle la forma de `OperationReservationStore` — el único mecanismo de
reclamo concurrente probado en este workspace — pero con clave `(projection_id, tag, tenant)` y un
ciclo de vida pensado para un bucle de sondeo continuo, no para un comando de una sola vez.

**Pregunta abierta para `sdd-design`**: puerto nuevo vs. `OffsetStore` evolucionado; el conjunto
exacto de métodos (un ilustrativo `try_claim`/`renew`/`complete`/`release` es una pista, no un
mandato); y si un advisory lock de PG o `FOR UPDATE SKIP LOCKED` supera a una tabla de reclamo para
este modelo de bucle de sondeo.

## Semántica Requerida

```
Dados dos trabajadores sondeando el mismo (projection_id, tag, tenant)
Cuando ambos intentan adquirir el reclamo al mismo tiempo
Entonces exactamente uno DEBE obtenerlo; el otro DEBE ser rechazado, y el
        trabajador rechazado NO DEBE llamar a fetch ni invocar el handler para
        ese flujo en ese tick.

Dado un trabajador que sostiene un reclamo válido sobre un flujo
Cuando aún está procesando un lote largo y su lease se acerca a la expiración
Entonces DEBE poder extender el lease y continuar, y ningún otro trabajador
        puede tomar el control del flujo mientras ese lease siga siendo válido.

Dado un trabajador que adquirió un reclamo y luego se detuvo — cayó, fue
      terminado, o quedó pausado indefinidamente — sin liberarlo
Cuando su lease expira
Entonces otro trabajador DEBE poder tomar el control del flujo sin intervención
        del operador y sin esperar indefinidamente, de modo que un trabajador
        muerto no pueda bloquear un flujo para siempre.

Dado un trabajador cuyo reclamo fue tomado por otro trabajador después de que
      su lease expirara
Cuando ese primer trabajador reanuda e intenta escribir estado de offset o de
      dedup como propietario
Entonces la escritura DEBE ser rechazada como propietario obsoleto y DEBE dejar
        el estado almacenado sin modificar — en particular NO DEBE retroceder un
        offset que el nuevo propietario ya avanzó.

Dado un trabajador que sostiene un reclamo válido
Cuando termina su lote y libera el reclamo normalmente
Entonces el flujo DEBE volver a ser reclamable de inmediato, sin esperar a que
        el lease expire.

Dada una composición que declara Profile::Production y que registra progreso de
      lado de lectura pero ningún mecanismo de reclamo durable
Cuando se llama a build()
Entonces DEBE ser rechazada en tiempo de composición/arranque — nunca diferida
        al primer sondeo ni al primer lote — con un error que nombre la
        capacidad faltante y la llamada exacta que lo corrige.

Dada una composición que declara Profile::Production y que registra un mecanismo
      de reclamo durable
Cuando se llama a build()
Entonces DEBE tener éxito, y el lado de lectura multi-réplica pasa a estar
        soportado bajo la restricción operativa establecida.

Dado un flujo cuyo reclamo es sostenido por un trabajador
Cuando ese trabajador procesa un lote
Entonces los eventos DEBEN seguir manejándose en orden ascendente de versión por
        (tenant, tag), exactamente como antes de este cambio — el reclamo NO DEBE
        reordenar, intercalar ni omitir eventos dentro de un flujo.

Dado un único trabajador que sostiene un reclamo válido durante todo el lote
Cuando cae después de que el handler tuvo éxito pero antes de que el lote quede
      completamente registrado
Entonces el handler PUEDE ejecutarse de nuevo para esos eventos al reanudar.
        Este cambio NO lo previene, y ningún artefacto entregado puede describirlo
        como procesamiento exactamente-una-vez ni efectos externos
        exactamente-una-vez.
```

## Áreas Afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistence-api/src/read_side/` | Nuevo | Puerto de reclamo (IS-2, IS-3) |
| `crates/domain/src/read_side/session.rs` | Modificado | El reclamo envuelve el lote (IS-4) |
| `crates/persistence/src/postgres/` + `migrations/016+` | Nuevo | Adaptador durable + tabla (IS-5, D-6) |
| Compuerta de composición de `crates/service-sdk` | Modificado | Producción falla cerrado (IS-6) |
| `crates/runtime/src/read_side/scheduler.rs` | Modificado | Ciclo de vida del reclamo entre sondeos |
| `examples/reference-app/src/read_side/mod.rs:118-126` | Modificado | Retirar la promesa PROD-014C |
| `integration-tests/tests/infrastructure/` | Nuevo | Suite de contención (IS-7) |
| `openspec/specs/{read-side-event-claiming,read-side}/spec.md` | Nuevo / Modificado | IS-8 |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| R-1 | "Reclamo atómico" se lee como "exactamente-una-vez" | Alta | OOS-2 es criterio de aceptación (SC-6); verificar por grep el cambio entregado, como hizo la pasada de verify de PROD-014B |
| R-2 | Un lease demasiado corto desaloja a un trabajador lento pero vivo a mitad de lote | Media | Renovación durante lotes largos + el fencing rechaza las escrituras tardías del desalojado (IS-3) |
| R-3 | El costo del reclamo por tick de sondeo degrada el rendimiento | Media | Reclamar una vez por flujo y mantenerlo durante el lote, no por evento |
| R-4 | Tocar `session.rs` + scheduler + compuerta + adaptador excede el presupuesto de 400 líneas | Alta | `sdd-tasks` pronostica rebanadas apiladas: puerto + adaptador, luego cableado de session/scheduler, luego compuerta + docs |
| R-5 | La desviación de reloj entre réplicas desajusta la expiración | Media | `Clock` inyectado, nunca `now()` de la base — precedente AD-8 de `reservation.rs` |

## Plan de Reversión

Aditivo a nivel de puerto y tabla. Revertir = eliminar el puerto de reclamo y el adaptador,
restaurar `session.rs` a la secuencia sin protección, revertir la compuerta de Producción, eliminar
la migración `016+`, y restaurar la restricción de adopción de escritor único en especificaciones y
documentación. La tabla de reclamo no es referenciada por nada más y puede eliminarse o dejarse en
su lugar; descartarla degrada el comportamiento de vuelta al de PROD-014B, no a corrupción.

## Dependencias

- PROD-014A / PROD-014B (archivados) — la compuerta de durabilidad y el par de progreso durable,
  consumidos.
- `crates/persistence/src/postgres/reservation.rs` — solo precedente de forma SQL, no se importa.
- `ego_integration_tests::isolated_database()`.
- Ninguna dependencia externa, crate o servicio nuevo.

## Criterios de Éxito

- [ ] **SC-1** — Dos trabajadores concurrentes sobre un `(projection_id, tag, tenant)`: exactamente
      uno mantiene un reclamo válido; el otro es rechazado y no invoca el handler.
- [ ] **SC-2** — Un trabajador que muere sosteniendo un reclamo lo libera por expiración; otro
      trabajador toma el control sin acción del operador y sin esperar indefinidamente.
- [ ] **SC-3** — Un trabajador cuyo reclamo fue tomado por otro no puede escribir estado de offset
      ni de dedup como propietario.
- [ ] **SC-4** — `Profile::Production` rechaza una composición de lado de lectura sin mecanismo de
      reclamo durable, y tiene éxito con uno.
- [ ] **SC-5** — El orden ascendente por `(tag, tenant)` y las garantías de durabilidad de
      PROD-014B quedan sin cambios; `cargo test --workspace` no muestra fallos nuevos.
- [ ] **SC-6** — Ningún artefacto entregado describe esto como procesamiento exactamente-una-vez ni
      efectos externos exactamente-una-vez; la documentación declara multi-réplica como soportado
      bajo la restricción operativa establecida.
- [ ] **SC-7** — La contención se prueba contra PostgreSQL real con múltiples contendientes
      concurrentes en `integration-tests/`, nunca como simulación en pruebas unitarias.
