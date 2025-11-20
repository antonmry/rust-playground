

## Testing

Traces:

```bash
# Traces with gRPC
docker run --rm ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  traces \
  --otlp-endpoint=host.docker.internal:4317 \
  --otlp-insecure \
  --duration=30s \
  --rate=5 \
  --service="test-service"
```

```bash
# HTTP/protobuf via telemetrygen
docker run --rm ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  traces \
  --otlp-endpoint=host.docker.internal:4318 \
  --otlp-http \
  --otlp-insecure \
  --duration=30s \
  --rate=5 \
  --service="test-service-http"
```

Metrics:

```bash
# gRPC
docker run --rm ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  metrics \
  --otlp-endpoint=host.docker.internal:4317 \
  --otlp-insecure \
  --duration=30s \
  --otlp-metric-name="demo_metric" \
  --rate=10

# HTTP/protobuf
docker run --rm ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  metrics \
  --otlp-endpoint=host.docker.internal:4318 \
  --otlp-http \
  --otlp-insecure \
  --duration=30s \
  --otlp-metric-name="demo_metric_http" \
  --rate=10
```

Logs:

```bash
# gRPC
docker run --rm ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  logs \
  --otlp-endpoint=host.docker.internal:4317 \
  --otlp-insecure \
  --duration=30s \
  --rate=5

# HTTP/protobuf
docker run --rm ghcr.io/open-telemetry/opentelemetry-collector-contrib/telemetrygen:latest \
  logs \
  --otlp-endpoint=host.docker.internal:4318 \
  --otlp-http \
  --otlp-insecure \
  --duration=30s \
  --rate=5
```

For HTTP/JSON tests, you can POST directly:

```bash
# Traces
curl -X POST http://localhost:4318/v1/traces \
  -H 'Content-Type: application/json' \
  -d '{
    "resourceSpans": [{
      "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "test-service-http"}}]},
      "scopeSpans": [{
        "scope": {"name": "demo"},
        "spans": [{
          "traceId": "00000000000000000000000000000001",
          "spanId": "0000000000000001",
          "name": "demo-span",
          "startTimeUnixNano": "1723500000000000000",
          "endTimeUnixNano": "1723500001000000000"
        }]
      }]
    }]
  }'

# Metrics (gauge)
curl -X POST http://localhost:4318/v1/metrics \
  -H 'Content-Type: application/json' \
  -d '{
    "resourceMetrics": [{
      "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "demo-metrics-http"}}]},
      "scopeMetrics": [{
        "metrics": [{
          "name": "demo_metric_http",
          "unit": "1",
          "gauge": {"dataPoints": [{
            "timeUnixNano": "1723500002000000000",
            "asDouble": 1.23
          }]}
        }]
      }]
    }]
  }'

# Logs
curl -X POST http://localhost:4318/v1/logs \
  -H 'Content-Type: application/json' \
  -d '{
    "resourceLogs": [{
      "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "demo-logs-http"}}]},
      "scopeLogs": [{
        "logRecords": [{
          "timeUnixNano": "1723500003000000000",
          "severityText": "INFO",
          "body": {"stringValue": "hello from curl"}
        }]
      }]
    }]
  }'
```
