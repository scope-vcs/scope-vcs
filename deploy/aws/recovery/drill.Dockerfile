# Recovery drill tools: PostgreSQL 18 clients for restore.py and the pinned
# recovery requirements. It runs only inside the drill's network-less namespace.
FROM postgres:18.6@sha256:86c951e05bf56c93d95d397747fb8820ac76cc3bedb78f43abd83eedbe3666ae
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 python3-venv \
    && rm -rf /var/lib/apt/lists/*
COPY requirements.txt /tmp/requirements.txt
RUN python3 -m venv /opt/recovery \
    && /opt/recovery/bin/pip install --no-cache-dir --require-hashes -r /tmp/requirements.txt \
    && rm /tmp/requirements.txt
ENV PATH=/opt/recovery/bin:$PATH
