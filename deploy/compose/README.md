# Laboratorio local de Fase 1 con Docker Compose

Este laboratorio levanta tres contenedores con el agente Rust para pruebas locales.

Para validar el escenario mixto de Fase 1 (3 agentes en Docker + 1 agente local en Windows), usa la seccion **Experimento mixto 4 nodos** de este mismo documento.

## Arranque

```bash
cd deploy/compose
docker compose up --build -d
```

## Verificacion

```bash
docker compose ps
docker compose logs -f agent-a
```

## Parada

```bash
docker compose down
```

## Limpieza completa (incluye volumenes)

```bash
docker compose down -v
```

## Nodos

- `agent-a`: Tier 0, scheduler+executor.
- `agent-b`: Tier 1, executor.
- `agent-c`: Tier 1, executor.

Los `agent.toml` estan en `deploy/compose/configs/`.

## Experimento mixto 4 nodos (3 Docker + 1 Windows local)

### Objetivo

Cerrar los pendientes de Fase 1 en entorno Windows:

- Cola/retry con `tier_min: 2` usando un cuarto nodo que aparece despues.
- Verificacion de limite de memoria Docker en Windows (`resources.memory_mb`) con evidencia local.

### Topologia

- Docker:
  - `agent-a` (Tier 0, scheduler+executor)
  - `agent-b` (Tier 1, executor)
  - `agent-c` (Tier 1, executor)
- Windows local:
  - `agent-windows-local` (Tier 2, executor)

### Preparacion

1. Arranca compose:

```bash
docker compose -f deploy/compose/docker-compose.yml up --build -d
```

Automatizacion opcional desde PowerShell:

```powershell
./scripts/phase1-mixed-experiment.ps1
```

1. Verifica convergencia inicial en `agent-a`:

```bash
curl -s -H "X-All4One-Secret: compose-secret" http://localhost:7946/v1/nodes
```

Debe devolver `total: 3` y `online: 3`.

1. Arranca el agente local Windows con `deploy/compose/configs/agent-win-local.toml`.

Si tienes binario local disponible, puedes arrancarlo desde el script:

```powershell
./scripts/phase1-mixed-experiment.ps1 -StartLocal -AgentExePath C:/ruta/all4one-agent.exe
```

El script publica `ALL4ONE_ADVERTISE_HOST=host.docker.internal` para que el nodo local anuncie un endpoint alcanzable desde los contenedores.

### Prueba A: Cola y retry con Tier 2

1. Con solo `agent-a/b/c` levantados, envia un job con `tier_min: 2`.
2. Verifica que queda en `status: queued`.
3. Arranca `agent-windows-local`.
4. Verifica transicion `queued -> running` en menos de 15s.

Payload sugerido:

```yaml
runtime: executable
source: cmd
command: ["/C", "echo hello-from-tier2"]
resources:
  cpu_cores: 1
  memory_mb: 128
constraints:
  tier_min: 2
```

### Prueba B: Limite de memoria Docker en Windows

1. Envia un job `runtime: docker` con `resources.memory_mb: 512`.
2. Confirma que el job se asigna al nodo Windows (`assigned_to` del nodo local).
3. Captura evidencia del limite de memoria desde Windows (Process Explorer o equivalente de Job Object).

Payload sugerido:

```yaml
runtime: docker
source: mcr.microsoft.com/powershell:lts-nanoserver-ltsc2022
command: ["pwsh", "-NoLogo", "-Command", "Start-Sleep -Seconds 40"]
resources:
  cpu_cores: 1
  memory_mb: 512
constraints:
  tier_min: 2
  requires_capabilities:
    docker: true
```

### Nota de alcance

Este documento define la secuencia del experimento. El estado de cierre de Fase 1 se mantiene en `docs/phases/phase-1.md` para evitar duplicacion de criterios.
