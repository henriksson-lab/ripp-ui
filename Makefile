local:
	cargo run --release --bin ripp

server:
	cargo run --release --bin ripp-server

gitaddall:
	git add src ui


loc:
	find src tests mm-demo/src -name '*.rs' | xargs wc -l
