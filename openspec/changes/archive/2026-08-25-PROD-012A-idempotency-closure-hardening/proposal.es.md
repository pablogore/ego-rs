# Propuesta: PROD-012A — Endurecimiento de Cierre de Idempotencia

## Por Qué Esto Es Su Propio Cambio

PROD-012 ("Procesamiento Idempotente de Comandos de Extremo a Extremo") fue
archivado como **Entregado** el 2026-08-20
(`openspec/changes/archive/2026-08-20-prod-012-idempotent-command-processing/`,
promovido a `openspec/specs/idempotent-command-processing/spec.md`). Un
cambio archivado es historia congelada, no un documento de trabajo. Cuando
una auditoría reciente contra una garantía ya "Entregada" encuentra brechas
reales, el movimiento correcto es un nuevo cambio de seguimiento atómico, no
una edición retroactiva de la carpeta cerrada de PROD-012 ni de su reporte de
archivo. Este es ese seguimiento.

## Qué Se Auditó

Una auditoría del 2026-08-25 reexaminó la garantía de PROD-012 de extremo a
extremo: comparación de huella (fingerprint), recibos `NoEvents`, CAS de
fencing, recuperación tras caída dual-agregado, extracción de clave neutral
al protocolo, y aislamiento de tenant/agregado. El mecanismo central se
sostuvo — está probado contra PostgreSQL real donde PROD-012 afirmaba que lo
estaba. El trabajo de la auditoría era encontrar dónde la prueba era más
delgada que la afirmación.

## Qué Se Encontró

Un bypass estructural y tres brechas de cobertura de pruebas, todas ahora
cerradas:

1. **Bypass estructural.** `#[operation]` fija `mutating: true` de forma
   codificada (`crates/service-sdk-macros/src/lib.rs:258-259`), y
   `#[idempotent]` era un atributo completamente opcional — nada a nivel de
   SDK exigía `mutating ⇒ idempotent`. Solo existía una prueba más estrecha,
   específica de la aplicación de referencia.
2. **Brecha de carrera multi-nodo.** La prueba existente de réplicas
   concurrentes probaba el fencing de propiedad de la reserva contra
   Postgres real, pero las escrituras reales de eventos/agregados de cada
   réplica pasaban por un almacén en memoria privado, así que "dos nodos
   compitiendo por confirmar la misma operación, solo sobrevive una
   escritura durable" nunca se probó de extremo a extremo.
3. **Brecha de recuperación tras caída de un solo agregado.** La
   recuperación tras caída-después-del-commit estaba probada solo para el
   caso dual-agregado; el caso de un solo agregado — un proceso real
   eliminado (killed) después de un commit real, luego recuperado por un
   proceso nuevo — no tenía prueba equivalente.
4. **Brecha de alcance de aislamiento.** El aislamiento entre
   tenant/tipo/id para los recibos estaba probado a nivel
   estructural/catálogo, nunca funcionalmente contra recibos reales de
   Postgres variando un campo de identidad a la vez.

Un quinto ítem se corrigió como desvío de documentación, no como brecha de
código: el ROADMAP y la especificación afirmaban "dos adaptadores
conformes — HTTP y gRPC" para el dispatch de comandos. Solo HTTP despacha
comandos reales; el adaptador gRPC es solo de portador/extracción
(`GrpcMetadataCarrier` pasa el arnés compartido para leer la clave de los
metadatos, pero no existe en el workspace ninguna ruta de servicio, socket o
dispatch de comandos gRPC — `crates/transport/src/lib.rs:10-32`).

## Por Qué Esto Es Atómico

Este cambio no añade ninguna capacidad nueva ni alcance nuevo. Cierra
brechas contra una garantía ya aceptada: un nuevo lint estructural, tres
nuevas pruebas de integración contra Postgres real, y correcciones de
documentación para que coincida con lo que el código realmente demuestra.
No se tomó ninguna decisión arquitectónica nueva — `design.md`/`decisions.md`
están deliberadamente ausentes de esta carpeta de cambio. Lo que sigue
abierto (atomicidad de la escritura dual, primer-valor-gana en claves
duplicadas, el arnés genérico de conformidad de reservas no parametrizado
contra Postgres, y el valor por defecto silencioso en memoria de
`EntityRuntimeBuilder`) permanece abierto, documentado como tal, y queda
explícitamente fuera de alcance aquí.
