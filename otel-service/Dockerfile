FROM rust:1.91

# Install system dependencies required to build Rust projects and run Node CLI tools
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev \
        ca-certificates \
        curl \
        gnupg \
        golang && \
    mkdir -p /etc/apt/keyrings && \
    curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg && \
    echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_20.x nodistro main" > /etc/apt/sources.list.d/nodesource.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends nodejs && \
    GO111MODULE=on go install github.com/open-telemetry/opentelemetry-collector-contrib/cmd/telemetrygen@latest && \
    ln -s /root/go/bin/telemetrygen /usr/local/bin/telemetrygen && \
    rm -rf /var/lib/apt/lists/*

ENV PATH="/root/go/bin:${PATH}"

# Install OpenAI Codex CLI from npm registry
RUN npm install -g @openai/codex

CMD ["telemetrygen"]
