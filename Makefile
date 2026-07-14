# Makefile — convenience targets for the two-stack setup.
# Prerequisites: infra stack must be up before running `make up`.

.PHONY: infra-up infra-down infra-logs infra-pull up down logs stop-all searxng-config

## SearXNG runtime config — gitignored (holds a live Brave api_key).
## Docker bind-mounts it, and would silently create a *directory* in its place
## if it were missing, so infra-up bootstraps it from the tracked template.
searxng-config:
	@test -f config/searxng/settings.yml || { \
	  cp config/searxng/settings.yml.example config/searxng/settings.yml; \
	  echo "Created config/searxng/settings.yml from template — add your Brave api_key."; \
	}

## Infra stack (llama-cpp, tei-embed, tei-rerank, searxng)
infra-up: searxng-config
	docker compose -f infra/docker-compose.yml --env-file infra/.env up -d

infra-down:
	docker compose -f infra/docker-compose.yml --env-file infra/.env down

infra-logs:
	docker compose -f infra/docker-compose.yml --env-file infra/.env logs -f

infra-pull:
	docker compose -f infra/docker-compose.yml --env-file infra/.env pull

## Researcher app
up:
	docker compose up -d

down:
	docker compose down

logs:
	docker compose logs -f

## Bring everything down (keeps volumes)
stop-all: down infra-down
