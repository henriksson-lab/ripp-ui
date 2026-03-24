local:
	cargo run --release 

server:
	cargo run --release -- --server

gitaddall:
	git add src ui assets


loc:
	find src tests -name '*.rs' | xargs wc -l
	find assets -name '*.wgsl' | xargs wc -l



#- cargo run — desktop app (default)                                                                                                                                   
#  - cargo run -- --server — web server mode
#  - cargo run -- --server --fps — server with FPS counter                                                                                                               
#  - cargo run -- --sim-camera — desktop with simulated camera        
