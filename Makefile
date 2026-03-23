local:
	cargo run --release --bin ripp

server:
	cargo run --release --bin ripp-server

gitaddall:
	git add src ui assets


loc:
	find src tests -name '*.rs' | xargs wc -l
	find assets -name '*.wgsl' | xargs wc -l

