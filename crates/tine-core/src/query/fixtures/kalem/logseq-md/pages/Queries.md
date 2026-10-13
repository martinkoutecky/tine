- Open tasks of the plugins
	- {{query (and [[Project/Plugins]] (task NOW LATER TODO DOING))}}
	  query-table:: true
	  query-properties:: [:block :page]
- The coming week: {{query (and (task NOW LATER TODO DOING) (between today +7d))}}
- The pages under Project: {{query (namespace [[Project]])}}
- An advanced query
	- #+BEGIN_QUERY
	  {:title "Done" :query [:find (pull ?b [*]) :where [?b :block/marker "DONE"]]}
	  #+END_QUERY
- Steps
  logseq.order-list-type:: number
	- One
	  logseq.order-list-type:: number
	- Two
	  logseq.order-list-type:: number
- #+BEGIN_NOTE
  An admonition naming [[Kalem]]
  #+END_NOTE
- #+BEGIN_QUOTE
  A quotation
  #+END_QUOTE
- A tag of two words: #[[note graphs]] and ^^a highlight^^ and $$x^2$$
- {{renderer :wordcount}} {{video https://example.com/v}}
- [[a [[nested]] link]]
