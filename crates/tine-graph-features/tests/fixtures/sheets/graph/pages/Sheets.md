public:: true

- Tasks
  tine.view:: table
  tine.fields:: price=number;qty=number
  tine.formula.total:: price * qty
  tine.col-aggregates:: prop:price=sum;formula:total=sum
  - TODO [#A] First #x
    price:: 5
    qty:: 2
  - DONE Second
    price:: 1
    qty:: 2
  - Third <script>alert(1)</script> row
    price:: 3
    background-color:: yellow
- Board
  tine.view:: board
  tine.group-by:: state
  - TODO Write tests
  - DOING Implement
  - Plain card
- Grid
  tine.view:: grid
  tine.header:: true
  tine.col-aggregates:: 1=sum
  -
    - Name
    - Qty
  -
    - a
    - 2
  -
    - b
    - 5
- Not a sheet
  tine.view:: nonsense
  - stays outline
