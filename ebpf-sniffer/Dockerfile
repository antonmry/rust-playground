# syntax=docker/dockerfile:1.6

FROM fedora:40 AS builder

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:$PATH

RUN dnf -y upgrade \
    && dnf -y install --setopt=install_weak_deps=False --setopt=tsflags=nodocs \
        clang \
        llvm \
        llvm-devel \
        elfutils-libelf-devel \
        pkgconf-pkg-config \
        git \
        curl \
        make \
        which \
        findutils \
        perl \
    && dnf clean all

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable \
    && rustup toolchain install nightly --component rust-src \
    && cargo install --locked bpf-linker

WORKDIR /workspace

COPY Cargo.toml Cargo.toml
COPY .cargo .cargo
COPY ebpf-sniffer/Cargo.toml ebpf-sniffer/Cargo.toml
COPY ebpf-sniffer/build.rs ebpf-sniffer/build.rs
COPY ebpf-sniffer/src ebpf-sniffer/src
COPY ebpf-sniffer-ebpf/Cargo.toml ebpf-sniffer-ebpf/Cargo.toml
COPY ebpf-sniffer-ebpf/src ebpf-sniffer-ebpf/src
COPY ebpf-sniffer-ebpf/bpfel-unknown-none.json ebpf-sniffer-ebpf/bpfel-unknown-none.json

RUN cargo generate-lockfile \
    && cargo update -p aya-log-common --precise 0.1.15 \
    && cargo fetch --locked

COPY . .

RUN cargo +nightly build --release --locked \
    --manifest-path ebpf-sniffer-ebpf/Cargo.toml \
    -Z build-std=core \
    -Z build-std-features=compiler-builtins-mem,compiler-builtins-no-f16-f128 \
    --target ebpf-sniffer-ebpf/bpfel-unknown-none.json

RUN cargo build --release --locked --manifest-path ebpf-sniffer/Cargo.toml

FROM fedora:40 AS runtime

RUN dnf -y upgrade \
    && dnf -y install --setopt=install_weak_deps=False --setopt=tsflags=nodocs \
        iproute \
        iproute-tc \
        iputils \
    && dnf clean all

COPY --from=builder /workspace/target/release/ebpf-sniffer /usr/local/bin/ebpf-sniffer

ENTRYPOINT ["/usr/local/bin/ebpf-sniffer"]
CMD ["--help"]
