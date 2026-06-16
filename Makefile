SHELL := bash
.DEFAULT_GOAL := help
.PHONY: help install update bin ext uninstall watch check

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

install: ## Full install (binary + extension + config + hook + enable)
	./scripts/install.sh

update: ## Deploy local changes (binary live now; extension after relogin)
	./scripts/install.sh --bin --ext

bin: ## Rebuild + reinstall the binary only (live instantly)
	./scripts/install.sh --bin

ext: ## Reinstall the GNOME extension only (relogin notice if changed)
	./scripts/install.sh --ext

uninstall: ## Remove everything (config preserved)
	./scripts/uninstall.sh

watch: ## Auto-rebuild + reinstall binary on every source save (needs cargo-watch)
	@command -v cargo-watch >/dev/null 2>&1 || { echo "cargo-watch not found. Install with: cargo install cargo-watch"; exit 1; }
	cargo watch -w src -w Cargo.toml -s './scripts/install.sh --bin'

check: ## Run the update-flow test suite
	@bash tests/test_update_flow.sh
