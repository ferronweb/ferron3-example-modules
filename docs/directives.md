# Example directives

Configuration reference for the example modules. Use `ferron directives` on the
fixture binary to see them as JSON.

## HTTP stages

### `example_header <value>`

Append `X-Example-Header: <value>` to every response. Valid in any `http`
block (global, host, location).

```ferron
example.com {
    example_header hello-world
}
```

### `hello_path <path>` / `hello_message <text>`

Serve a plain-text greeting for the configured path.

```ferron
example.com {
    hello_path /greet
    hello_message "Greetings!"
}
```

## Custom server

### `echo_server { listen <addr> }`

Run a TCP echo server. Global only.

```ferron
{
    echo_server {
        listen "127.0.0.1:9090"
    }
}
```

## Observability

### `observability { provider memory; memory { max_events <n> } }`

Use the in-memory sink. Keeps the last `max_events` events (default 1000).

```ferron
example.com {
    observability {
        provider memory
        memory {
            max_events 500
        }
    }
}
```

## TLS

### `tls { provider selfsigned; selfsigned { days <n> } }`

Generate a self-signed certificate for the host.

```ferron
example.com {
    tls {
        provider selfsigned
        selfsigned {
            days 30
        }
    }
}
```

## DNS

### `dns memory`

In-memory DNS provider for ACME DNS-01 (example only, no persistence).

```ferron
example.com {
    tls {
        provider acme
        acme {
            dns memory
        }
    }
}
```

## Configuration adapter

### `--config-adapter toml --config-params file=...`

Load configuration from TOML. Also auto-selected for `.toml` files.

```bash
ferron run --config-adapter toml --config-params file=ferron.toml
```

The example adapter accepts an optional `json` key inside the TOML that holds
a JSON-encoded `ServerConfiguration` for real use.

## Log formatter

### `format csv` + `csv { fields ... }`

Format access logs as CSV.

```ferron
example.com {
    observability {
        provider console
        console {
            format csv
            csv {
                fields method path status
            }
        }
    }
}
```
