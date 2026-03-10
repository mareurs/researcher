# Makefile — convenience targets for the two-stack setup.
# Prerequisites: infra stack must be up before running `make up`.

.PHONY: infra-up infra-down infra-logs infra-pull up down logs stop-all

## Infra stack (llama-cpp, tei-embed, tei-rerank, searxng)
infra-up:
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
