; JSX, after the script's own query: the elements' tags and attributes.

(jsx_opening_element
  name: (identifier) @tag)
(jsx_closing_element
  name: (identifier) @tag)
(jsx_self_closing_element
  name: (identifier) @tag)

(jsx_opening_element
  name: (member_expression) @tag)
(jsx_closing_element
  name: (member_expression) @tag)
(jsx_self_closing_element
  name: (member_expression) @tag)

(jsx_attribute
  (property_identifier) @attribute)

(jsx_opening_element
  ["<" ">"] @punctuation.bracket)
(jsx_closing_element
  ["</" ">"] @punctuation.bracket)
(jsx_self_closing_element
  ["<" "/>"] @punctuation.bracket)
