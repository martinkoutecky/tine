public:: true

- {{query (task TODO DOING)}}
  tine.view:: table
  tine.fields:: qty=number
- {{query (task TODO DOING DONE)}}
  tine.view:: board
  tine.group-by:: state
- {{query (task TODO)}}
  tine.view:: grid
  -
    - stays a result list
