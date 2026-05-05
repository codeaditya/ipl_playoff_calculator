FROM quay.io/pypa/manylinux_2_28_x86_64:latest

ENV RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:${PATH}

RUN dnf -y update && \
    dnf -y install curl rsync git gcc gcc-c++ make pkgconf-pkg-config openssl-devel perl && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
      sh -s -- -y \
        --profile minimal \
        --default-toolchain stable \
        --no-modify-path && \
    dnf clean all

WORKDIR /work
