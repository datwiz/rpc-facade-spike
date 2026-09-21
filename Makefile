# Every target here is meant to be runnable on a laptop with no AWS account.
# The two that are not (lambda-build, lambda-deploy) say so.

VENV ?= .venv
PYTHON ?= $(VENV)/bin/python
MATURIN ?= $(VENV)/bin/maturin

# Deploy knobs; override on the command line, e.g.
#   make lambda-deploy FUNCTION=something-poc ALIAS=live
FUNCTION ?= something-poc
ALIAS ?= live
IAM_ROLE ?=

.PHONY: help
help:
	@grep -E '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | sed 's/:.*## /\t/'

.PHONY: test
test: ## Run the Rust suite (no AWS, no network at run time)
	cargo test

.PHONY: fmt
fmt: ## Format
	cargo fmt --all

.PHONY: lint
lint: ## Format check + clippy, warnings are errors
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings
	cargo clippy -p something-client --no-default-features --all-targets -- -D warnings
	cargo clippy -p something-py --all-targets -- -D warnings

.PHONY: check
check: lint test ## What to run before pushing

.PHONY: venv
venv: ## Create the dev virtualenv
	python3 -m venv $(VENV)
	$(PYTHON) -m pip install --quiet --upgrade pip maturin pytest

.PHONY: py-develop
py-develop: ## Build the native module into the virtualenv
	VIRTUAL_ENV=$(CURDIR)/$(VENV) $(MATURIN) develop

.PHONY: py-test
py-test: ## Run the Python suite (requires py-develop first)
	$(PYTHON) -m pytest tests/python -q

.PHONY: lambda-build
lambda-build: ## Cross-compile the Lambda binary (needs cargo-lambda + zig)
	cargo lambda build --release --arm64 -p something-lambda

.PHONY: lambda-watch
lambda-watch: ## Run the handler locally on :9000 for cargo lambda invoke
	cargo lambda watch

# Publishes a numbered version and points the alias at it. Invoking through an
# alias is what keeps register and exec on the same code.
.PHONY: lambda-deploy
lambda-deploy: lambda-build ## Deploy, publish a version, move the alias
	cargo lambda deploy $(FUNCTION) \
		$(if $(IAM_ROLE),--iam-role $(IAM_ROLE),) \
		--binary-name bootstrap \
		--enable-function-url=false
	aws lambda publish-version --function-name $(FUNCTION) --query Version --output text \
		| xargs -I {} sh -c 'aws lambda update-alias --function-name $(FUNCTION) --name $(ALIAS) --function-version {} \
		  || aws lambda create-alias --function-name $(FUNCTION) --name $(ALIAS) --function-version {}'

.PHONY: clean
clean:
	cargo clean
	rm -rf $(VENV) python/something/_something*.so
