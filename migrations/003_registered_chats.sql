CREATE TABLE IF NOT EXISTS registered_chats (
    chat_id BIGINT PRIMARY KEY,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
